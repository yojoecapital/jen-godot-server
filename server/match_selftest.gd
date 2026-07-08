extends SceneTree


var _fails := 0


func _initialize() -> void:
	var db_path := OS.get_environment("DB_PATH")
	if db_path.is_empty():
		db_path = "user://match_selftest.db"
	DirAccess.remove_absolute(db_path)

	var db := JenDb.new()
	_check(db.open(db_path), "db opens")
	var reg := MatchRegistry.new(db)

	var created := reg.create("alice", _config(12345))
	_check(created["ok"], "create -> ok")
	var code := str(created["code"])
	_eq(created["current_seat"], 0, "seat 0 acts first")
	_eq(created["seats"], ["human", "cpu"], "seats normalized (ai->cpu)")
	_check(created["snapshot"].has("rng_state"), "snapshot carries rng_state for client sync")
	_eq(reg.owner_of(code), "alice", "ownership tracked")
	_eq(reg.human_seats(code), [0], "only seat 0 is human")

	# wrong seat is rejected; the acting unit must belong to the seat
	_eq(reg.apply(code, 1, Action.end_turn().to_dict())["reason"], "not_your_turn", "seat 1 can't act on seat 0's turn")

	# seat 0 ends turn; the CPU (seat 1) then auto-plays back to seat 0 (or a winner)
	var res := reg.apply(code, 0, Action.end_turn().to_dict())
	_check(res["ok"], "seat 0 END_TURN applied")
	_check((res["actions"] as Array).size() >= 1, "action stream returned")
	_eq(res["actions"][0]["seat"], 0, "human action broadcast first")
	var saw_cpu := false
	for a in res["actions"]:
		if int(a["seat"]) == 1:
			saw_cpu = true
	_check(saw_cpu, "server drove the CPU seat")
	_check(res["current_seat"] == 0 or int(res["winner"]) != -1, "turn returns to the human (or match ended)")
	var hash_after: String = res["state_hash"]

	# determinism: same seed + same human action stream => identical state hash
	var reg2 := MatchRegistry.new(db)
	var created2 := reg2.create("bob", _config(12345))
	var res2 := reg2.apply(str(created2["code"]), 0, Action.end_turn().to_dict())
	_eq(res2["state_hash"], hash_after, "deterministic replay: identical seed+actions => identical state")

	# persistence: drop from memory, rehydrate from persistence, hash must survive
	reg.drop_match(code)
	var resumed := reg.view(code)
	_check(resumed["ok"], "match rehydrated from persistence after eviction")
	# recompute the live hash on the resumed match via another no-op path
	_check(resumed["snapshot"].has("rng_state"), "resumed snapshot keeps rng_state")
	var resumed_hash := str(hash(JSON.stringify(resumed["snapshot"])))
	_eq(resumed_hash, hash_after, "resumed state hashes identically (durable + deterministic)")

	# unknown match
	_eq(reg.view("ZZZZ")["ok"], false, "unknown code -> not ok")

	db.close()
	print("\n[match_selftest] %s  (%d failing)" % ["PASS" if _fails == 0 else "FAIL", _fails])
	quit(1 if _fails > 0 else 0)


func _config(seed: int) -> GameConfig:
	var c := GameConfig.new()
	c.dim = 8
	c.compact_spawn = true
	c.player_colors = [Color.MAGENTA, Color.CYAN]
	c.seat_controllers = ["human", "ai"]
	c.seed = seed
	c.seeded = true
	return c


func _check(cond: bool, label: String) -> void:
	print(("  ok  " if cond else "  FAIL ") + label)
	if not cond:
		_fails += 1


func _eq(actual: Variant, expected: Variant, label: String) -> void:
	var ok: bool = actual == expected
	print(("  ok  " if ok else "  FAIL ") + label + ("" if ok else "  (got %s, want %s)" % [actual, expected]))
	if not ok:
		_fails += 1
