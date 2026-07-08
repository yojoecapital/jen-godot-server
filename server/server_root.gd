extends Node


var _db: JenDb
var _registry: MatchRegistry
var _admin_api: AdminApi
var _admin_http: AdminHttpServer
var _ws_gateway: WsGateway


func _ready() -> void:
	var admin_secret := OS.get_environment("ADMIN_API_SECRET")
	if admin_secret.is_empty():
		push_warning("ADMIN_API_SECRET is not set — the admin API will reject every request.")
	var db_path := _env("DB_PATH", "user://jen.db")
	var admin_port := int(_env("ADMIN_PORT", "8080"))
	var ws_port := int(_env("WS_PORT", "8081"))

	_db = JenDb.new()
	if not _db.open(db_path):
		push_error("Could not open the persistence database at %s" % db_path)
		get_tree().quit(1)
		return
	print("[jen-server] db ready at ", db_path)

	_registry = MatchRegistry.new(_db)

	# Held as members so the router Callable's target isn't freed when _ready returns.
	_admin_api = AdminApi.new(_db, admin_secret, _registry)
	_admin_http = AdminHttpServer.new()
	_admin_http.router = _admin_api.handle
	if _admin_http.start(admin_port) != OK:
		push_error("Admin HTTP server could not bind port %d" % admin_port)
		get_tree().quit(1)
		return
	print("[jen-server] admin REST on :", admin_port)

	_ws_gateway = WsGateway.new(_db, _registry)
	if _ws_gateway.start(ws_port) != OK:
		push_error("WebSocket gateway could not bind port %d" % ws_port)
		get_tree().quit(1)
		return
	print("[jen-server] ws gameplay on :", ws_port)


func _process(_delta: float) -> void:
	if _admin_http != null:
		_admin_http.poll()
	if _ws_gateway != null:
		_ws_gateway.poll()


func _notification(what: int) -> void:
	if what == NOTIFICATION_WM_CLOSE_REQUEST or what == NOTIFICATION_PREDELETE:
		if _admin_http != null:
			_admin_http.stop()
		if _ws_gateway != null:
			_ws_gateway.stop()
		if _db != null:
			_db.close()


static func _env(key: String, fallback: String) -> String:
	var v := OS.get_environment(key)
	return v if not v.is_empty() else fallback
