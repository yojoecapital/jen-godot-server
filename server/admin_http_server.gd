class_name AdminHttpServer
extends RefCounted


const _MAX_REQUEST_BYTES := 1 << 20   # 1 MiB guard
const _CONN_TIMEOUT_MS := 5000

var router: Callable   # func(method: String, path: String, headers: Dictionary, body: String) -> Dictionary

var _tcp := TCPServer.new()
var _conns: Array = []


func start(port: int) -> Error:
	return _tcp.listen(port)


func stop() -> void:
	_tcp.stop()
	_conns.clear()


func poll() -> void:
	while _tcp.is_connection_available():
		var peer := _tcp.take_connection()
		_conns.append({"peer": peer, "buf": PackedByteArray(), "since": Time.get_ticks_msec()})
	var keep: Array = []
	for c in _conns:
		if _service(c):
			keep.append(c)
	_conns = keep


func _service(c: Dictionary) -> bool:
	var peer: StreamPeerTCP = c["peer"]
	peer.poll()
	var status := peer.get_status()
	if status == StreamPeerTCP.STATUS_ERROR or status == StreamPeerTCP.STATUS_NONE:
		return false
	if status == StreamPeerTCP.STATUS_CONNECTING:
		return Time.get_ticks_msec() - int(c["since"]) < _CONN_TIMEOUT_MS
	var avail := peer.get_available_bytes()
	if avail > 0:
		var got: Array = peer.get_partial_data(avail)
		if got[0] == OK:
			c["buf"].append_array(got[1])
	var buf: PackedByteArray = c["buf"]
	if buf.size() > _MAX_REQUEST_BYTES:
		_respond(peer, 413, {"error": "payload_too_large"})
		return false
	var req := _try_parse(buf)
	if req.is_empty():
		return Time.get_ticks_msec() - int(c["since"]) < _CONN_TIMEOUT_MS
	var result: Dictionary = router.call(req["method"], req["path"], req["headers"], req["body"]) if router.is_valid() else {"status": 500, "json": {"error": "no_router"}}
	_respond(peer, int(result.get("status", 200)), result.get("json", {}))
	return false


# Returns {} until a full request is buffered, else {method, path, headers, body}.
func _try_parse(buf: PackedByteArray) -> Dictionary:
	var text := buf.get_string_from_utf8()
	var head_end := text.find("\r\n\r\n")
	if head_end == -1:
		return {}
	var head := text.substr(0, head_end)
	var lines := head.split("\r\n")
	if lines.is_empty():
		return {}
	var request_line := lines[0].split(" ")
	if request_line.size() < 2:
		return {}
	var headers := {}
	for i in range(1, lines.size()):
		var idx := lines[i].find(":")
		if idx > 0:
			headers[lines[i].substr(0, idx).strip_edges().to_lower()] = lines[i].substr(idx + 1).strip_edges()
	var content_length := int(headers.get("content-length", "0"))
	var body_start := head_end + 4
	var body_bytes := buf.slice(body_start)
	if body_bytes.size() < content_length:
		return {}  # wait for the rest of the body
	return {
		"method": request_line[0].to_upper(),
		"path": request_line[1],
		"headers": headers,
		"body": body_bytes.slice(0, content_length).get_string_from_utf8(),
	}


func _respond(peer: StreamPeerTCP, status: int, json: Variant) -> void:
	var body := JSON.stringify(json).to_utf8_buffer()
	var reason := _reason(status)
	var head := "HTTP/1.1 %d %s\r\n" % [status, reason]
	head += "Content-Type: application/json\r\n"
	head += "Content-Length: %d\r\n" % body.size()
	head += "Connection: close\r\n\r\n"
	peer.put_data(head.to_utf8_buffer())
	peer.put_data(body)
	peer.disconnect_from_host()


static func _reason(status: int) -> String:
	match status:
		200: return "OK"
		201: return "Created"
		400: return "Bad Request"
		401: return "Unauthorized"
		403: return "Forbidden"
		404: return "Not Found"
		409: return "Conflict"
		413: return "Payload Too Large"
		_: return "Internal Server Error"
