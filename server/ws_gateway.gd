class_name WsGateway
extends RefCounted

## Plain-WebSocket gameplay gateway. Accepts raw TCP connections and upgrades each
## with WebSocketPeer.accept_stream (so it pairs with the client's WebSocketPeer),
## authenticates by API-key secret -> scopes, claims a human seat per client, and
## routes JSON messages to the MatchRegistry. Broadcasts the registry's authoritative
## action stream to every client in a match; clients replay it deterministically.

var _db: JenDb
var _registry: MatchRegistry
var _tcp := TCPServer.new()
var _sessions: Array = []   # each: {ws, authed, key_id, scopes, code, seat}


func _init(db: JenDb, registry: MatchRegistry) -> void:
	_db = db
	_registry = registry


func start(port: int) -> Error:
	return _tcp.listen(port)


func stop() -> void:
	_tcp.stop()
	_sessions.clear()


func poll() -> void:
	while _tcp.is_connection_available():
		var conn := _tcp.take_connection()
		var ws := WebSocketPeer.new()
		ws.accept_stream(conn)
		_sessions.append({"ws": ws, "authed": false, "key_id": "", "scopes": [], "code": "", "seat": -1})
	var keep: Array = []
	for s in _sessions:
		var ws: WebSocketPeer = s["ws"]
		ws.poll()
		var st := ws.get_ready_state()
		if st == WebSocketPeer.STATE_OPEN:
			while ws.get_available_packet_count() > 0:
				_handle(s, ws.get_packet().get_string_from_utf8())
			keep.append(s)
		elif st == WebSocketPeer.STATE_CONNECTING:
			keep.append(s)
		else:
			_detach(s)   # closing/closed -> release seat
	_sessions = keep


func _handle(s: Dictionary, text: String) -> void:
	var msg = JSON.parse_string(text)
	if typeof(msg) != TYPE_DICTIONARY:
		return
	var t := str(msg.get("t", ""))
	if not s["authed"]:
		if t == "auth":
			_authenticate(s, msg)
		return
	match t:
		"create_match": _create_match(s, msg)
		"join_match": _join_match(s, str(msg.get("code", "")).to_upper())
		"list_matches": _send(s, {"t": "matches", "matches": _joinable_list()})
		"delete_match": _delete_match(s, str(msg.get("code", "")).to_upper())
		"action": _action(s, msg.get("action", {}))
		"leave": _detach(s)
		"save": _send(s, {"t": "saved", "code": s["code"]})
		_: _send(s, {"t": "error", "message": "unknown_message"})


func _authenticate(s: Dictionary, msg: Dictionary) -> void:
	var key := str(msg.get("key", ""))
	var id := str(msg.get("id", ""))
	var row := _db.get_key_by_secret_hash(JenAuth.hash_secret(key))
	if row.is_empty() or (not id.is_empty() and str(row.get("id", "")) != id):
		_send(s, {"t": "hello", "ok": false})
		return
	var scopes = JSON.parse_string(str(row.get("scopes", "[]")))
	s["authed"] = true
	s["key_id"] = str(row.get("id", ""))
	s["scopes"] = scopes if scopes is Array else []
	_send(s, {"t": "hello", "ok": true, "scopes": s["scopes"], "id": s["key_id"]})


func _create_match(s: Dictionary, msg: Dictionary) -> void:
	if "host_match" not in s["scopes"]:
		_send(s, {"t": "error", "message": "forbidden_host"})
		return
	var config := GameConfig.from_dict(msg.get("config", {}))
	var res := _registry.create(str(s["key_id"]), config)
	if not res.get("ok", false):
		_send(s, {"t": "error", "message": str(res.get("reason", "create_failed"))})
		return
	_attach_and_start(s, str(res["code"]), res)


func _join_match(s: Dictionary, code: String) -> void:
	var res := _registry.view(code)
	if not res.get("ok", false):
		_send(s, {"t": "error", "message": "no_match"})
		return
	_attach_and_start(s, code, res)


func _attach_and_start(s: Dictionary, code: String, res: Dictionary) -> void:
	var seat := _claim_seat(code, s)
	if seat == -1:
		_send(s, {"t": "error", "message": "match_full"})
		return
	s["code"] = code
	s["seat"] = seat
	_send(s, {
		"t": "match_start",
		"code": code,
		"seed": res.get("seed", 0),
		"yourSeat": seat,
		"seats": res.get("seats", []),
		"snapshot": res.get("snapshot", {}),
	})


func _action(s: Dictionary, action_dict: Variant) -> void:
	var code := str(s["code"])
	if code.is_empty():
		_send(s, {"t": "error", "message": "not_in_match"})
		return
	var res := _registry.apply(code, int(s["seat"]), action_dict if action_dict is Dictionary else {})
	if not res.get("ok", false):
		_send(s, {"t": "error", "message": str(res.get("reason", "rejected"))})
		return
	for a in res["actions"]:
		_broadcast(code, {"t": "action", "seat": a["seat"], "action": a["action"], "state_hash": res["state_hash"]})
	if int(res.get("winner", -1)) != -1:
		_broadcast(code, {"t": "game_over", "winner": res["winner"]})


func _delete_match(s: Dictionary, code: String) -> void:
	if _registry.owner_of(code) != str(s["key_id"]):
		_send(s, {"t": "error", "message": "forbidden_delete"})
		return
	_registry.drop_match(code)
	_db.delete_match(code)
	_broadcast(code, {"t": "room_closed"})
	for other in _sessions:
		if str(other["code"]) == code:
			other["code"] = ""
			other["seat"] = -1


# Lowest human seat not already held by a live session in this match.
func _claim_seat(code: String, s: Dictionary) -> int:
	var claimed := {}
	for other in _sessions:
		if other != s and str(other["code"]) == code and int(other["seat"]) >= 0:
			claimed[int(other["seat"])] = true
	for seat in _registry.human_seats(code):
		if not claimed.has(seat):
			return seat
	return -1


func _detach(s: Dictionary) -> void:
	s["code"] = ""
	s["seat"] = -1


func _joinable_list() -> Array:
	var out: Array = []
	for entry in _registry.list():
		if entry.get("status", "") != "open":
			continue
		var code := str(entry["code"])
		var humans: int = _registry.human_seats(code).size()
		var claimed := 0
		for s in _sessions:
			if str(s["code"]) == code and int(s["seat"]) >= 0:
				claimed += 1
		entry["open_seats"] = maxi(0, humans - claimed)
		out.append(entry)
	return out


func _send(s: Dictionary, msg: Dictionary) -> void:
	var ws: WebSocketPeer = s["ws"]
	if ws.get_ready_state() == WebSocketPeer.STATE_OPEN:
		ws.send_text(JSON.stringify(msg))


func _broadcast(code: String, msg: Dictionary) -> void:
	var text := JSON.stringify(msg)
	for s in _sessions:
		if str(s["code"]) == code:
			var ws: WebSocketPeer = s["ws"]
			if ws.get_ready_state() == WebSocketPeer.STATE_OPEN:
				ws.send_text(text)
