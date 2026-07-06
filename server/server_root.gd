extends Node

## Entry point for the Jen dedicated server. Boots the SQLite store, the admin
## REST API, and (Phase 2) the WebSocket gameplay gateway, then pumps them every
## frame. Configuration comes entirely from environment variables so the same
## build runs unchanged in Docker.
##   ADMIN_API_SECRET  (required) bearer token for the admin REST API
##   DB_PATH           SQLite file path            (default: user://jen.db)
##   ADMIN_PORT        admin REST port             (default: 8080)
##   WS_PORT           gameplay WebSocket port     (default: 8081)

var _db: JenDb
var _admin_api: AdminApi
var _admin_http: AdminHttpServer
var _registry   # MatchRegistry (Phase 2)
var _ws_gateway # WsGateway (Phase 2)


func _ready() -> void:
	var admin_secret := OS.get_environment("ADMIN_API_SECRET")
	if admin_secret.is_empty():
		push_warning("ADMIN_API_SECRET is not set — the admin API will reject every request.")
	var db_path := _env("DB_PATH", "user://jen.db")
	var admin_port := int(_env("ADMIN_PORT", "8080"))
	var ws_port := int(_env("WS_PORT", "8081"))

	_db = JenDb.new()
	if not _db.open(db_path):
		push_error("Could not open the SQLite database at %s" % db_path)
		get_tree().quit(1)
		return
	print("[jen-server] db ready at ", db_path)

	# Phase 2 assigns _registry (the gameplay layer); admin match-delete tolerates null.
	# Held as a member so the router Callable's target isn't freed when _ready returns.
	_admin_api = AdminApi.new(_db, admin_secret, _registry)
	_admin_http = AdminHttpServer.new()
	_admin_http.router = _admin_api.handle
	var err := _admin_http.start(admin_port)
	if err != OK:
		push_error("Admin HTTP server could not bind port %d (err %d)" % [admin_port, err])
		get_tree().quit(1)
		return
	print("[jen-server] admin REST on :", admin_port)
	print("[jen-server] ws gameplay port reserved :", ws_port, " (Phase 2)")


func _process(_delta: float) -> void:
	if _admin_http != null:
		_admin_http.poll()
	if _ws_gateway != null:
		_ws_gateway.poll()


func _notification(what: int) -> void:
	if what == NOTIFICATION_WM_CLOSE_REQUEST or what == NOTIFICATION_PREDELETE:
		if _admin_http != null:
			_admin_http.stop()
		if _db != null:
			_db.close()


static func _env(key: String, fallback: String) -> String:
	var v := OS.get_environment(key)
	return v if not v.is_empty() else fallback
