class_name JenDb
extends Object

## Thin SQLite wrapper (godot-sqlite GDExtension). Owns two tables:
##   api_keys — admin-issued client credentials (id, secret hash, scopes)
##   matches  — persisted authoritative match state (JSON snapshot + metadata)
## All writes are parameterized; secrets are stored only as SHA-256 hex.

var _db: SQLite


func open(path: String) -> bool:
	_db = SQLite.new()
	_db.path = path
	_db.verbosity_level = SQLite.QUIET
	if not _db.open_db():
		return false
	_db.query("PRAGMA journal_mode=WAL;")
	_db.query("""CREATE TABLE IF NOT EXISTS api_keys (
		id TEXT PRIMARY KEY,
		secret_hash TEXT NOT NULL,
		scopes TEXT NOT NULL,
		created_at INTEGER NOT NULL
	);""")
	_db.query("""CREATE TABLE IF NOT EXISTS matches (
		code TEXT PRIMARY KEY,
		owner_key_id TEXT NOT NULL,
		seed INTEGER NOT NULL,
		seats TEXT NOT NULL,
		snapshot TEXT NOT NULL,
		status TEXT NOT NULL,
		created_at INTEGER NOT NULL,
		updated_at INTEGER NOT NULL
	);""")
	return true


func close() -> void:
	if _db != null:
		_db.close_db()


# ---- api_keys ----

func insert_key(id: String, secret_hash: String, scopes: Array) -> bool:
	return _db.query_with_bindings(
		"INSERT INTO api_keys (id, secret_hash, scopes, created_at) VALUES (?, ?, ?, ?);",
		[id, secret_hash, JSON.stringify(scopes), _now()])


func key_exists(id: String) -> bool:
	_db.query_with_bindings("SELECT 1 FROM api_keys WHERE id = ?;", [id])
	return not _db.query_result.is_empty()


func get_key_by_id(id: String) -> Dictionary:
	_db.query_with_bindings(
		"SELECT id, scopes, created_at FROM api_keys WHERE id = ?;", [id])
	return _row(_db.query_result)


func get_key_by_secret_hash(secret_hash: String) -> Dictionary:
	_db.query_with_bindings(
		"SELECT id, scopes, created_at FROM api_keys WHERE secret_hash = ?;", [secret_hash])
	return _row(_db.query_result)


func list_keys() -> Array:
	_db.query("SELECT id, scopes, created_at FROM api_keys ORDER BY created_at DESC;")
	return _db.query_result.duplicate()


func delete_key(id: String) -> bool:
	if not key_exists(id):
		return false
	return _db.query_with_bindings("DELETE FROM api_keys WHERE id = ?;", [id])


# ---- matches ----

func upsert_match(code: String, owner_key_id: String, seed: int, seats: Array,
		snapshot: Dictionary, status: String) -> bool:
	return _db.query_with_bindings(
		"""INSERT INTO matches (code, owner_key_id, seed, seats, snapshot, status, created_at, updated_at)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?)
		ON CONFLICT(code) DO UPDATE SET snapshot=excluded.snapshot, status=excluded.status,
			seats=excluded.seats, updated_at=excluded.updated_at;""",
		[code, owner_key_id, seed, JSON.stringify(seats), JSON.stringify(snapshot), status, _now(), _now()])


func get_match(code: String) -> Dictionary:
	_db.query_with_bindings("SELECT * FROM matches WHERE code = ?;", [code])
	return _row(_db.query_result)


func list_matches(open_only := false) -> Array:
	if open_only:
		_db.query("SELECT code, owner_key_id, seats, status, updated_at FROM matches WHERE status = 'open' ORDER BY updated_at DESC;")
	else:
		_db.query("SELECT code, owner_key_id, seats, status, updated_at FROM matches ORDER BY updated_at DESC;")
	return _db.query_result.duplicate()


func delete_match(code: String) -> bool:
	_db.query_with_bindings("SELECT 1 FROM matches WHERE code = ?;", [code])
	if _db.query_result.is_empty():
		return false
	return _db.query_with_bindings("DELETE FROM matches WHERE code = ?;", [code])


# ---- helpers ----

static func _now() -> int:
	return int(Time.get_unix_time_from_system())


static func _row(result: Array) -> Dictionary:
	return result[0] if not result.is_empty() else {}
