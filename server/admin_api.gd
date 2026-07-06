class_name AdminApi
extends RefCounted

## Admin REST surface, authenticated by the ADMIN_API_SECRET bearer token.
##   POST   /admin/keys           {id, scopes:[...]}  -> {id, secret, scopes}   (secret shown once)
##   GET    /admin/keys                               -> {keys:[...]}
##   DELETE /admin/keys/{id}                           -> {ok}
##   GET    /admin/matches                            -> {matches:[...]}
##   DELETE /admin/matches/{code}                      -> {ok}
## Match state lives in the registry (Phase 2); admin can delete any match.

var _db: JenDb
var _secret: String
var _registry   # MatchRegistry, injected in Phase 2 (may be null for key-only ops)


func _init(db: JenDb, admin_secret: String, registry = null) -> void:
	_db = db
	_secret = admin_secret
	_registry = registry


func handle(method: String, path: String, headers: Dictionary, body: String) -> Dictionary:
	if not _authorized(headers):
		return _err(401, "unauthorized")
	var segments := _segments(path)
	# /admin/keys ...
	if segments.size() >= 2 and segments[0] == "admin" and segments[1] == "keys":
		if segments.size() == 2 and method == "POST":
			return _create_key(body)
		if segments.size() == 2 and method == "GET":
			return _ok({"keys": _db.list_keys()})
		if segments.size() == 3 and method == "DELETE":
			return _ok({"ok": _db.delete_key(segments[2])})
	# /admin/matches ...
	if segments.size() >= 2 and segments[0] == "admin" and segments[1] == "matches":
		if segments.size() == 2 and method == "GET":
			return _ok({"matches": _db.list_matches()})
		if segments.size() == 3 and method == "DELETE":
			var code: String = segments[2]
			if _registry != null:
				_registry.drop_match(code)
			return _ok({"ok": _db.delete_match(code)})
	return _err(404, "not_found")


func _create_key(body: String) -> Dictionary:
	var parsed = JSON.parse_string(body)
	if typeof(parsed) != TYPE_DICTIONARY:
		return _err(400, "bad_json")
	var id := str(parsed.get("id", "")).strip_edges()
	if id.is_empty():
		return _err(400, "missing_id")
	var scopes := JenAuth.normalize_scopes(parsed.get("scopes", []))
	if scopes.is_empty():
		return _err(400, "no_valid_scopes")
	if _db.key_exists(id):
		return _err(409, "id_exists")
	var secret := JenAuth.generate_secret()
	if not _db.insert_key(id, JenAuth.hash_secret(secret), scopes):
		return _err(500, "db_error")
	return {"status": 201, "json": {"id": id, "secret": secret, "scopes": scopes}}


func _authorized(headers: Dictionary) -> bool:
	if _secret.is_empty():
		return false
	var auth := str(headers.get("authorization", ""))
	var prefix := "Bearer "
	if not auth.begins_with(prefix):
		return false
	return JenAuth.constant_time_equals(auth.substr(prefix.length()), _secret)


static func _segments(path: String) -> Array:
	var clean := path.split("?", true, 1)[0]
	var out: Array = []
	for s in clean.split("/"):
		if not s.is_empty():
			out.append(s.uri_decode())
	return out


static func _ok(json: Variant) -> Dictionary:
	return {"status": 200, "json": json}


static func _err(status: int, code: String) -> Dictionary:
	return {"status": status, "json": {"error": code}}
