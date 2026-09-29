import "./style.css";

const app = document.querySelector("#app");
let parcels = [];
let selected = null;
let activeView = "contract";
let busy = false;

const escapeHtml = (value) => String(value ?? "").replace(/[&<>'"]/g, (character) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" })[character]);
const status = (value) => `<span class="status status-${escapeHtml(value)}"><i></i>${escapeHtml(value)}</span>`;
const code = (value) => `<code>${escapeHtml(value)}</code>`;

async function request(path, options) {
  const response = await fetch(path, options);
  const payload = await response.json();
  if (!response.ok) throw new Error(payload.error || "Local request failed");
  return payload;
}

function render() {
  const selectedMarkup = selected ? detailMarkup(selected) : `<section class="empty"><h1>No local parcels yet</h1><p>Capture a backend failure through the CLI, FastAPI adapter, or MCP server. It will appear here from the same local store.</p></section>`;
  app.innerHTML = `
    <div class="app-frame">
      <header class="topbar">
        <div class="brand"><img src="/bugparcel-mark.png" alt="" /><span>BugParcel</span><b>LOCAL</b></div>
        <div class="connection"><i></i><span>mcp://bugparcel</span><em>127.0.0.1</em></div>
      </header>
      <main class="workspace">
        <aside class="rail">
          <div class="rail-title"><div><p>INCIDENTS</p><strong>${parcels.length} parcel${parcels.length === 1 ? "" : "s"}</strong></div><button id="refresh" title="Refresh local store" aria-label="Refresh local store">↻</button></div>
          <div class="parcel-list">${parcels.length ? parcels.map((parcel) => `
            <button class="parcel ${parcel.parcel_id === selected?.parcel_id ? "selected" : ""}" data-parcel="${escapeHtml(parcel.parcel_id)}">
              <span class="parcel-main"><b>${escapeHtml(parcel.branch_hint || "Detached source")}</b>${status(parcel.status)}</span>
              <span class="parcel-id">${escapeHtml(parcel.parcel_id)}</span>
              <span class="parcel-meta">${escapeHtml(new Date(parcel.created_at).toLocaleString())}</span>
            </button>`).join("") : `<p class="rail-empty">The local store is empty.</p>`}</div>
          <footer><i></i>Detached replays only</footer>
        </aside>
        ${selectedMarkup}
      </main>
    </div>`;
  bindEvents();
}

function detailMarkup(parcel) {
  const failure = parcel.reproduction.failure_assertion;
  const state = parcel.reproduction.state;
  const fixtures = state?.fixtures ?? [];
  const events = parcel.status_events ?? [];
  const body = activeView === "contract" ? `
    <section class="contract">
      <div class="failure"><span>Expected failure</span><strong>${escapeHtml(failure.expected_output_contains.join(" · ") || "non-zero exit")}</strong><em>exit ${escapeHtml(failure.expected_exit_code)}</em></div>
      <div class="command"><span>$</span>${code(parcel.reproduction.command.join(" "))}</div>
      <div class="split">
        <div><h2>Captured state</h2><pre>${escapeHtml(state ? JSON.stringify(state.json, null, 2) : "No request state captured.")}</pre></div>
        <div><h2>Environment</h2><dl><dt>Python</dt><dd>${escapeHtml(parcel.reproduction.environment.python?.version || "Not recorded")}</dd><dt>Container</dt><dd>${escapeHtml(parcel.reproduction.environment.container_image || "Host replay")}</dd><dt>Network</dt><dd>Disabled for Docker replays</dd></dl></div>
      </div>
    </section>` : activeView === "history" ? `<ol class="history">${events.map((event) => `<li><i></i><div><strong>${escapeHtml(event.to)}</strong><p>${escapeHtml(event.reason || "State transition")}</p></div><time>${escapeHtml(new Date(event.at).toLocaleTimeString())}</time></li>`).join("") || "<p>No state transitions recorded.</p>"}</ol>` : `<section class="files"><h2>Sealed material</h2>${fixtures.length ? fixtures.map((fixture) => `<div>${code(fixture.relative_path)}<span>${escapeHtml(fixture.sha256.slice(0, 12))}…</span></div>`).join("") : "<p>No file fixtures attached.</p>"}</section>`;
  return `
    <section class="detail">
      <header class="detail-head"><div><p>PARCEL</p><h1>${escapeHtml(parcel.parcel_id)}</h1><span>${escapeHtml(parcel.source.repository_path)} <b>·</b> ${escapeHtml(parcel.source.commit_sha.slice(0, 9))}</span></div>${status(parcel.status)}</header>
      <nav class="tabs"><button data-view="contract" class="${activeView === "contract" ? "active" : ""}">Contract</button><button data-view="history" class="${activeView === "history" ? "active" : ""}">History</button><button data-view="files" class="${activeView === "files" ? "active" : ""}">Files</button></nav>
      ${body}
      <footer class="actions"><span id="notice">${busy ? "Running isolated operation…" : "Source branch is read-only"}</span><div><button data-action="bugparcel_reproduce" ${busy ? "disabled" : ""}>Reproduce</button><button class="primary-action" data-action="bugparcel_diagnose" ${busy ? "disabled" : ""}>Diagnose</button></div></footer>
    </section>`;
}

function bindEvents() {
  document.querySelector("#refresh")?.addEventListener("click", loadParcels);
  document.querySelectorAll("[data-parcel]").forEach((button) => button.addEventListener("click", () => loadParcel(button.dataset.parcel)));
  document.querySelectorAll("[data-view]").forEach((button) => button.addEventListener("click", () => { activeView = button.dataset.view; render(); }));
  document.querySelectorAll("[data-action]").forEach((button) => button.addEventListener("click", () => runAction(button.dataset.action)));
}

async function loadParcels() {
  try {
    parcels = await request("/api/parcels");
    const next = parcels.find((parcel) => parcel.parcel_id === selected?.parcel_id) ?? parcels[0];
    selected = next ? await request(`/api/parcels/${encodeURIComponent(next.parcel_id)}`) : null;
    render();
  } catch (error) { app.innerHTML = `<main class="error"><h1>Local bridge unavailable</h1><p>${escapeHtml(error.message)}</p><code>npm run bridge</code></main>`; }
}

async function loadParcel(parcelId) {
  selected = await request(`/api/parcels/${encodeURIComponent(parcelId)}`);
  activeView = "contract";
  render();
}

async function runAction(tool) {
  busy = true;
  render();
  try {
    const result = await request("/api/actions", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ tool, parcel_id: selected.parcel_id }) });
    const notice = document.querySelector("#notice");
    if (notice) notice.textContent = tool === "bugparcel_diagnose" ? `Diagnosis worktree: ${result.diagnosis?.isolated_worktree || "ready"}` : `Replay ${result.replay?.matched ? "matched the contract" : "did not match"}`;
  } catch (error) { const notice = document.querySelector("#notice"); if (notice) notice.textContent = error.message; }
  busy = false;
  document.querySelectorAll("[data-action]").forEach((button) => { button.disabled = false; });
}

loadParcels();
