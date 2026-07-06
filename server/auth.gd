class_name JenAuth
extends Object


const VALID_SCOPES := ["host_match", "join_match"]


static func generate_secret() -> String:
	return Crypto.new().generate_random_bytes(24).hex_encode()


static func hash_secret(secret: String) -> String:
	return secret.sha256_text()


static func normalize_scopes(raw: Variant) -> Array:
	var out: Array = []
	if raw is Array:
		for s in raw:
			var scope := str(s)
			if scope in VALID_SCOPES and scope not in out:
				out.append(scope)
	return out


# Length-stable comparison so a mismatch reveals nothing via timing.
static func constant_time_equals(a: String, b: String) -> bool:
	var ba := a.to_utf8_buffer()
	var bb := b.to_utf8_buffer()
	var diff := ba.size() ^ bb.size()
	var n: int = maxi(ba.size(), bb.size())
	for i in n:
		var x: int = ba[i] if i < ba.size() else 0
		var y: int = bb[i] if i < bb.size() else 0
		diff |= x ^ y
	return diff == 0
