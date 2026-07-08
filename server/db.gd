class_name JenDb
extends Object


var _data_path: String = "res://data"
var _keys_file: String
var _matches_path: String


func open(path: String) -> bool:
	_data_path = path
	_keys_file = _data_path.path_join("keys.json")
	_matches_path = _data_path.path_join("matches")

	if not DirAccess.dir_exists_absolute(_data_path):
		DirAccess.make_dir_recursive_absolute(_data_path)

	if not DirAccess.dir_exists_absolute(_matches_path):
		DirAccess.make_dir_recursive_absolute(_matches_path)

	if not FileAccess.file_exists(_keys_file):
		_save_json(_keys_file, [])

	return true


func close() -> void:
	pass


# ---- api_keys ----


func insert_key(id: String, secret_hash: String, scopes: Array) -> bool:
	var keys := _load_json(_keys_file)

	for key in keys:
		if key.get("id") == id:
			return false

	keys.append({
		"id": id,
		"secret_hash": secret_hash,
		"scopes": scopes,
		"created_at": _now()
	})

	return _save_json(_keys_file, keys)


func key_exists(id: String) -> bool:
	return not get_key_by_id(id).is_empty()


func get_key_by_id(id: String) -> Dictionary:
	var keys := _load_json(_keys_file)

	for key in keys:
		if key.get("id") == id:
			return key

	return {}


func get_key_by_secret_hash(secret_hash: String) -> Dictionary:
	var keys := _load_json(_keys_file)

	for key in keys:
		if key.get("secret_hash") == secret_hash:
			return key

	return {}


func list_keys() -> Array:
	var keys := _load_json(_keys_file)
	keys.sort_custom(func(a, b):
		return a.get("created_at", 0) > b.get("created_at", 0)
	)

	return keys


func delete_key(id: String) -> bool:
	var keys := _load_json(_keys_file)
	var changed := false

	for i in range(keys.size() - 1, -1, -1):
		if keys[i].get("id") == id:
			keys.remove_at(i)
			changed = true

	if changed:
		return _save_json(_keys_file, keys)

	return false


# ---- matches ----


func upsert_match(code: String, owner_key_id: String, seed: int,
		seats: Array, snapshot: Dictionary, status: String) -> bool:

	var path := _match_file(code)
	var now := _now()

	var existing := {}

	if FileAccess.file_exists(path):
		existing = _load_json(path)

	var match_data := {
		"code": code,
		"owner_key_id": owner_key_id,
		"seed": seed,
		"seats": seats,
		"snapshot": snapshot,
		"status": status,
		"created_at": existing.get("created_at", now),
		"updated_at": now
	}

	return _save_json(path, match_data)


func get_match(code: String) -> Dictionary:
	var path := _match_file(code)

	if not FileAccess.file_exists(path):
		return {}

	return _load_json(path)


func list_matches(open_only := false) -> Array:
	var result: Array = []

	var dir := DirAccess.open(_matches_path)
	if dir == null:
		return result

	dir.list_dir_begin()

	while true:
		var file := dir.get_next()

		if file == "":
			break

		if dir.current_is_dir():
			continue

		if not file.ends_with(".json"):
			continue

		var match_data := _load_json(_matches_path.path_join(file))

		if open_only and match_data.get("status") != "open":
			continue

		result.append(match_data)

	dir.list_dir_end()

	result.sort_custom(func(a, b):
		return a.get("updated_at", 0) > b.get("updated_at", 0)
	)

	return result


func delete_match(code: String) -> bool:
	var path := _match_file(code)

	if not FileAccess.file_exists(path):
		return false

	DirAccess.remove_absolute(path)
	return true


# ---- helpers ----


func _match_file(code: String) -> String:
	return _matches_path.path_join(code + ".json")


func _load_json(path: String) -> Variant:
	if not FileAccess.file_exists(path):
		return []

	var file := FileAccess.open(path, FileAccess.READ)

	if file == null:
		return []

	var text := file.get_as_text()
	file.close()

	if text.is_empty():
		return []

	var parsed = JSON.parse_string(text)

	if parsed == null:
		return []

	return parsed


func _save_json(path: String, data: Variant) -> bool:
	var file := FileAccess.open(path, FileAccess.WRITE)

	if file == null:
		return false

	file.store_string(JSON.stringify(data, "\t"))
	file.close()

	return true


static func _now() -> int:
	return int(Time.get_unix_time_from_system())