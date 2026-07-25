"use strict";

// The signed-in key secret lives in localStorage; every /api call carries it as a bearer token.
const KEY = "jen_secret";
let me = null; // { id, scopes }

const $ = (id) => document.getElementById(id);

function secret() {
	return localStorage.getItem(KEY) || "";
}

async function api(method, path, body) {
	const opts = { method, headers: { "Authorization": "Bearer " + secret() } };
	if (body !== undefined) {
		opts.headers["Content-Type"] = "application/json";
		opts.body = JSON.stringify(body);
	}
	const res = await fetch(path, opts);
	let data = null;
	try { data = await res.json(); } catch (_) { data = null; }
	return { ok: res.ok, status: res.status, data };
}

function isAdmin() {
	return me && Array.isArray(me.scopes) && me.scopes.includes("admin");
}

function fmtTime(unix) {
	if (!unix) return "—";
	return new Date(unix * 1000).toLocaleString();
}

// ---- session ----

async function refreshMe() {
	const r = await api("GET", "/api/me");
	if (!r.ok) { me = null; return false; }
	me = r.data;
	return true;
}

function showSignedIn() {
	$("login").classList.add("hidden");
	$("whoami").classList.remove("hidden");
	$("whoami-id").textContent = me.id;
	$("whoami-scopes").textContent = (me.scopes || []).join(" · ");
	$("matches").classList.remove("hidden");
	$("matches-scope").textContent = isAdmin()
		? "Showing every match on the server."
		: "Showing matches you own.";
	if (isAdmin()) {
		$("clients").classList.remove("hidden");
		loadClients();
	} else {
		$("clients").classList.add("hidden");
	}
	loadMatches();
}

function showSignedOut() {
	me = null;
	localStorage.removeItem(KEY);
	$("whoami").classList.add("hidden");
	$("matches").classList.add("hidden");
	$("clients").classList.add("hidden");
	$("login").classList.remove("hidden");
}

async function signIn() {
	const s = $("login-secret").value.trim();
	$("login-error").textContent = "";
	if (!s) { $("login-error").textContent = "Enter a secret."; return; }
	localStorage.setItem(KEY, s);
	if (await refreshMe()) {
		showSignedIn();
	} else {
		localStorage.removeItem(KEY);
		$("login-error").textContent = "That secret was not accepted.";
	}
}

// ---- matches ----

async function loadMatches() {
	const r = await api("GET", "/api/matches");
	const body = $("matches-body");
	body.innerHTML = "";
	const rows = (r.ok && r.data && r.data.matches) || [];
	$("matches-empty").classList.toggle("hidden", rows.length > 0);
	for (const m of rows) {
		const tr = document.createElement("tr");
		tr.appendChild(cell(m.code, "mono"));
		tr.appendChild(cell(m.owner_key_id || "—"));
		tr.appendChild(cell(Array.isArray(m.seats) ? m.seats.length : "—"));
		tr.appendChild(cell(m.status));
		tr.appendChild(cell(fmtTime(m.updated_at)));
		tr.appendChild(actionCell("Delete", async () => {
			if (!confirm("Delete match " + m.code + "?")) return;
			await api("DELETE", "/api/matches/" + encodeURIComponent(m.code));
			loadMatches();
		}));
		body.appendChild(tr);
	}
}

// ---- clients (admin) ----

async function loadClients() {
	const r = await api("GET", "/api/keys");
	const body = $("clients-body");
	body.innerHTML = "";
	const rows = (r.ok && r.data && r.data.keys) || [];
	for (const k of rows) {
		const tr = document.createElement("tr");
		tr.appendChild(cell(k.id, "mono"));
		tr.appendChild(cell((k.scopes || []).join(" · ")));
		tr.appendChild(cell(fmtTime(k.created_at)));
		const disabled = k.id === me.id; // don't let admins delete the key they're using
		tr.appendChild(actionCell("Revoke", async () => {
			if (!confirm("Revoke key " + k.id + "?")) return;
			await api("DELETE", "/api/keys/" + encodeURIComponent(k.id));
			loadClients();
		}, disabled));
		body.appendChild(tr);
	}
}

async function createKey(ev) {
	ev.preventDefault();
	const id = $("key-id").value.trim();
	const scopes = Array.from(document.querySelectorAll("#create-key input[type=checkbox]:checked"))
		.map((c) => c.value);
	const r = await api("POST", "/api/keys", { id, scopes });
	if (!r.ok) {
		alert("Could not create key: " + (r.data && r.data.error || r.status));
		return;
	}
	$("key-id").value = "";
	$("new-secret-value").textContent = r.data.secret;
	$("new-secret").classList.remove("hidden");
	loadClients();
}

// ---- helpers ----

function cell(text, cls) {
	const td = document.createElement("td");
	td.textContent = text;
	if (cls) td.className = cls;
	return td;
}

function actionCell(label, onClick, disabled) {
	const td = document.createElement("td");
	const btn = document.createElement("button");
	btn.textContent = label;
	btn.className = "danger";
	btn.disabled = !!disabled;
	btn.addEventListener("click", onClick);
	td.appendChild(btn);
	return td;
}

// ---- wire up ----

$("login-btn").addEventListener("click", signIn);
$("login-secret").addEventListener("keydown", (e) => { if (e.key === "Enter") signIn(); });
$("logout").addEventListener("click", showSignedOut);
$("create-key").addEventListener("submit", createKey);
document.querySelectorAll("[data-refresh]").forEach((b) => {
	b.addEventListener("click", () => (b.dataset.refresh === "clients" ? loadClients() : loadMatches()));
});

(async function boot() {
	if (secret() && await refreshMe()) {
		showSignedIn();
	} else {
		showSignedOut();
	}
})();
