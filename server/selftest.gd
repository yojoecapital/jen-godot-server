extends SceneTree


var _fails := 0


func _initialize() -> void:
	var db_path := OS.get_environment("DB_PATH")
	if db_path.is_empty():
		db_path = "user://selftest.db"
	DirAccess.remove_absolute(db_path)

	var db := JenDb.new()
	_check(db.open(db_path), "db opens")

	var api := AdminApi.new(db, "s3cret", null)
	var H := {"authorization": "Bearer s3cret"}
	var BAD := {"authorization": "Bearer wrong"}

	var r := api.handle("POST", "/admin/keys", H, '{"id":"alice","scopes":["host_match","join_match"]}')
	_eq(r["status"], 201, "create alice -> 201")
	_check(str(r["json"].get("secret", "")).length() >= 32, "alice secret returned once")
	_eq((r["json"].get("scopes", []) as Array).size(), 2, "alice has 2 scopes")

	_eq(api.handle("POST", "/admin/keys", {}, '{"id":"bob"}')["status"], 401, "no auth -> 401")
	_eq(api.handle("POST", "/admin/keys", BAD, '{"id":"bob"}')["status"], 401, "wrong secret -> 401")
	_eq(api.handle("POST", "/admin/keys", H, '{"id":"alice","scopes":["host_match"]}')["status"], 409, "duplicate id -> 409")
	_eq(api.handle("POST", "/admin/keys", H, '{"id":"carol","scopes":["nonsense"]}')["status"], 400, "bad scope -> 400")
	_eq(api.handle("POST", "/admin/keys", H, 'not json')["status"], 400, "bad json -> 400")
	_eq(api.handle("POST", "/admin/keys", H, '{"id":"dave","scopes":["join_match"]}')["status"], 201, "create dave -> 201")

	var keys := api.handle("GET", "/admin/keys", H, "")
	_eq(keys["status"], 200, "list keys -> 200")
	_eq((keys["json"]["keys"] as Array).size(), 2, "two keys listed")

	_eq(api.handle("GET", "/admin/matches", H, "")["status"], 200, "list matches -> 200")
	_eq(api.handle("DELETE", "/admin/keys/alice", H, "")["json"]["ok"], true, "delete alice -> ok")
	_eq((api.handle("GET", "/admin/keys", H, "")["json"]["keys"] as Array).size(), 1, "one key after delete")
	_eq(api.handle("GET", "/admin/unknown", H, "")["status"], 404, "unknown route -> 404")

	# secret lookup path (used by the WS gateway in Phase 2)
	var dave_hash := JenAuth.hash_secret("whatever")
	_check(db.get_key_by_secret_hash(dave_hash).is_empty(), "unknown secret hash -> no key")

	# auth helpers
	_check(JenAuth.constant_time_equals("abc", "abc"), "const-time equal true")
	_check(not JenAuth.constant_time_equals("abc", "abcd"), "const-time equal false (len)")
	_check(JenAuth.hash_secret("x") == JenAuth.hash_secret("x"), "hash is deterministic")
	_eq(JenAuth.normalize_scopes(["host_match", "host_match", "bogus"]).size(), 1, "scopes dedup + filter")

	# HTTP request parsing
	var http := AdminHttpServer.new()
	var raw := "POST /admin/keys?x=1 HTTP/1.1\r\nHost: h\r\nContent-Length: 7\r\n\r\n{\"a\":1}".to_utf8_buffer()
	var parsed: Dictionary = http._try_parse(raw)
	_eq(parsed.get("method", ""), "POST", "parse method")
	_eq(parsed.get("path", ""), "/admin/keys?x=1", "parse path")
	_eq(parsed.get("body", ""), '{"a":1}', "parse body")
	_check(http._try_parse("GET / HTTP/1.1\r\nHost: h".to_utf8_buffer()).is_empty(), "incomplete request -> wait")
	_eq(AdminApi._segments("/admin/matches/AB12"), ["admin", "matches", "AB12"], "path segmentation")

	db.close()
	print("\n[selftest] %s  (%d failing)" % ["PASS" if _fails == 0 else "FAIL", _fails])
	quit(1 if _fails > 0 else 0)


func _check(cond: bool, label: String) -> void:
	print(("  ok  " if cond else "  FAIL ") + label)
	if not cond:
		_fails += 1


func _eq(actual: Variant, expected: Variant, label: String) -> void:
	var ok: bool = actual == expected
	print(("  ok  " if ok else "  FAIL ") + label + ("" if ok else "  (got %s, want %s)" % [actual, expected]))
	if not ok:
		_fails += 1
