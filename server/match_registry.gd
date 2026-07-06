class_name MatchRegistry
extends RefCounted


const CODE_CHARS := "ABCDEFGHJKMNPQRSTUVWXYZ23456789"
const CODE_LEN := 4
const CPU_ACTION_CAP := 2000

var _db: JenDb
var _matches := {}   # code -> {state, tm, rng, seats, owner, status}


func _init(db: JenDb) -> void:
	_db = db


func create(owner_key_id: String, config: GameConfig) -> Dictionary:
	if not config.seeded:
		config.seed = randi()
		config.seeded = true
	var rng := RandomNumberGenerator.new()
	rng.seed = config.seed
	var data := GameFactory.build(config, rng)
	var m := {
		"state": data["state"],
		"tm": data["turn_manager"],
		"rng": rng,
		"seats": _seats(config),
		"owner": owner_key_id,
		"status": "open",
	}
	var code := _new_code()
	_matches[code] = m
	_persist(code)
	return {
		"ok": true,
		"code": code,
		"seed": config.seed,
		"seats": m["seats"],
		"snapshot": _snapshot(m),
		"current_seat": _current_seat(m),
	}


func view(code: String) -> Dictionary:
	var m = _lookup(code)
	if m == null:
		return {"ok": false, "reason": "no_match"}
	return {
		"ok": true,
		"code": code,
		"seed": int(m["rng"].seed),
		"seats": m["seats"],
		"snapshot": _snapshot(m),
		"current_seat": _current_seat(m),
		"winner": _winner(m),
	}


# Applies one seat's action, then auto-runs any CPU seats that follow. Returns the
# ordered action stream for the gateway to broadcast (human action first).
func apply(code: String, seat: int, action_dict: Dictionary) -> Dictionary:
	var m = _lookup(code)
	if m == null:
		return {"ok": false, "reason": "no_match"}
	if _winner(m) != -1:
		return {"ok": false, "reason": "match_over"}
	if seat != _current_seat(m):
		return {"ok": false, "reason": "not_your_turn", "current_seat": _current_seat(m)}
	var action := Action.from_dict(action_dict)
	if not _actor_owned_by(m["state"], action, m["tm"].current_player()):
		return {"ok": false, "reason": "not_your_unit"}
	if not ActionExecutor.apply(m["state"], m["tm"], action, m["rng"]):
		return {"ok": false, "reason": "illegal_action"}
	var actions: Array = [{"seat": seat, "action": action.to_dict()}]
	actions.append_array(_run_cpu(m))
	if _winner(m) != -1:
		m["status"] = "over"
	_persist(code)
	return {
		"ok": true,
		"actions": actions,
		"current_seat": _current_seat(m),
		"winner": _winner(m),
		"state_hash": _hash(m),
	}


func list() -> Array:
	var out: Array = []
	for code in _matches:
		var m = _matches[code]
		out.append({
			"code": code,
			"seats": m["seats"],
			"owner": m["owner"],
			"status": m["status"],
			"current_seat": _current_seat(m),
			"winner": _winner(m),
		})
	return out


func owner_of(code: String) -> String:
	var m = _lookup(code)
	return str(m["owner"]) if m != null else ""


func has(code: String) -> bool:
	return _lookup(code) != null


func drop_match(code: String) -> void:
	_matches.erase(code)


func seat_is_cpu(code: String, seat: int) -> bool:
	var m = _lookup(code)
	return m != null and _seat_is_cpu(m, seat)


func human_seats(code: String) -> Array:
	var m = _lookup(code)
	if m == null:
		return []
	var out: Array = []
	for i in (m["seats"] as Array).size():
		if not _seat_is_cpu(m, i):
			out.append(i)
	return out


# ---- internals ----

# Returns the match from memory, rehydrating from the DB snapshot if needed.
func _lookup(code: String):
	if _matches.has(code):
		return _matches[code]
	var row := _db.get_match(code)
	if row.is_empty():
		return null
	var snapshot = JSON.parse_string(str(row.get("snapshot", "")))
	if typeof(snapshot) != TYPE_DICTIONARY:
		return null
	var loaded := SaveManager.deserialize(snapshot)
	var rng := RandomNumberGenerator.new()
	rng.seed = int(loaded.get("rng_seed", int(row.get("seed", 0))))
	if loaded.has("rng_state"):
		rng.state = int(loaded["rng_state"])
	var seats = JSON.parse_string(str(row.get("seats", "[]")))
	var m := {
		"state": loaded["state"],
		"tm": loaded["turn_manager"],
		"rng": rng,
		"seats": seats if seats is Array else [],
		"owner": str(row.get("owner_key_id", "")),
		"status": str(row.get("status", "open")),
	}
	_matches[code] = m
	return m


func _run_cpu(m: Dictionary) -> Array:
	var actions: Array = []
	var guard := 0
	while _winner(m) == -1 and _seat_is_cpu(m, _current_seat(m)) and guard < CPU_ACTION_CAP:
		guard += 1
		var seat := _current_seat(m)
		var action: Action = HeuristicPolicy.new().choose(m["state"], m["tm"])
		if not ActionExecutor.apply(m["state"], m["tm"], action, m["rng"]):
			break
		actions.append({"seat": seat, "action": action.to_dict()})
	return actions


func _persist(code: String) -> void:
	var m = _matches[code]
	_db.upsert_match(code, str(m["owner"]), int(m["rng"].seed), m["seats"], _snapshot(m), str(m["status"]))


func _snapshot(m: Dictionary) -> Dictionary:
	return SaveManager.serialize(m["state"], m["tm"], m["seats"], m["rng"])


func _seats(config: GameConfig) -> Array:
	var out: Array = []
	for kind in config.seat_controllers:
		out.append("cpu" if str(kind) in ["cpu", "ai"] else "human")
	if out.is_empty():
		for _i in maxi(config.player_colors.size(), 1):
			out.append("human")
	return out


func _seat_is_cpu(m: Dictionary, seat: int) -> bool:
	var seats: Array = m["seats"]
	return seat >= 0 and seat < seats.size() and seats[seat] == "cpu"


func _current_seat(m: Dictionary) -> int:
	if _winner(m) != -1:
		return -1
	var player: Player = m["tm"].current_player()
	return player.player_index if player != null else -1


func _winner(m: Dictionary) -> int:
	var player: Player = m["tm"].check_win()
	return player.player_index if player != null else -1


func _hash(m: Dictionary) -> String:
	return str(hash(JSON.stringify(_snapshot(m))))


func _actor_owned_by(state: GameState, action: Action, player: Player) -> bool:
	if action.kind == Action.Kind.END_TURN:
		return true
	if player == null:
		return false
	var occ = state.get_unit(action.actor_coord)
	if occ != null:
		return occ.player == player
	var piece = state.get_path_piece(action.actor_coord)
	return piece != null and piece.player == player


func _new_code() -> String:
	for _attempt in 100:
		var code := ""
		for _i in CODE_LEN:
			code += CODE_CHARS[randi() % CODE_CHARS.length()]
		if not _matches.has(code) and _db.get_match(code).is_empty():
			return code
	return "%08X" % (randi() & 0x7fffffff)
