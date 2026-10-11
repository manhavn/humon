<script lang="ts">
  import type { CollectionInfo } from "./api";
  import { app, type ConnNode } from "./state.svelte";
  import ConnectionForm from "./ConnectionForm.svelte";

  let adding = $state(false);
  /** Saved id awaiting a second click to confirm deletion. */
  let confirmDelete = $state<string | null>(null);

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const units = ["KB", "MB", "GB", "TB"];
    let v = bytes / 1024;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i++;
    }
    return `${v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
  }

  function remove(conn: ConnNode) {
    if (confirmDelete !== conn.saved.id) {
      confirmDelete = conn.saved.id;
      setTimeout(() => confirmDelete === conn.saved.id && (confirmDelete = null), 3000);
      return;
    }
    confirmDelete = null;
    app.removeConnection(conn).catch((e) => (conn.error = String(e)));
  }

  function openColl(e: MouseEvent, conn: ConnNode, db: string, coll: CollectionInfo) {
    // Cmd/Ctrl-click or middle-click opens another tab even if one exists.
    app.openCollection(conn, db, coll, e.metaKey || e.ctrlKey || e.button === 1);
  }
</script>

<aside>
  <header>
    <span>Bağlantılar</span>
    <button class="icon" onclick={() => (adding = !adding)} title="Yeni bağlantı">+</button>
  </header>

  {#if adding}
    <div class="add"><ConnectionForm oncancel={() => (adding = false)} /></div>
  {/if}

  {#if app.loadError}<p class="error">{app.loadError}</p>{/if}

  <ul class="tree">
    {#each app.conns as conn (conn.saved.id)}
      <li>
        <div class="row conn" class:connected={conn.connId}>
          <button class="main" onclick={() => app.toggleConnection(conn)} title={conn.saved.uri}>
            <span class="caret" class:open={conn.open}>▸</span>
            <span class="dot" class:on={conn.connId} class:busy={conn.connecting}></span>
            <span class="name">{conn.saved.name}</span>
          </button>
          <span class="actions">
            {#if conn.connId}
              <button class="icon" onclick={() => app.refreshDatabases(conn)} title="Yenile">↻</button>
              <button class="icon" onclick={() => app.disconnect(conn)} title="Bağlantıyı kes">⏏</button>
            {/if}
            <button
              class="icon"
              class:danger={confirmDelete === conn.saved.id}
              onclick={() => remove(conn)}
              title="Kaydı sil"
            >
              {confirmDelete === conn.saved.id ? "Sil?" : "×"}
            </button>
          </span>
        </div>
        {#if conn.connecting}<p class="hint">Bağlanıyor…</p>{/if}
        {#if conn.error}<p class="error">{conn.error}</p>{/if}

        {#if conn.open}
          <ul>
            {#each conn.dbs as db (db.name)}
              <li>
                <button class="row db" onclick={() => app.toggleDatabase(conn, db)}>
                  <span class="caret" class:open={db.open}>▸</span>
                  <span class="name">{db.name}</span>
                  <span class="meta">{formatSize(db.sizeOnDisk)}</span>
                </button>
                {#if db.open}
                  {#if db.loading}
                    <p class="hint deep">Yükleniyor…</p>
                  {:else if db.collections?.length === 0}
                    <p class="hint deep">Collection yok</p>
                  {:else}
                    <ul>
                      {#each db.collections ?? [] as c (c.name)}
                        {@const active =
                          app.activeTab?.connId === conn.connId &&
                          app.activeTab?.db === db.name &&
                          app.activeTab?.coll === c.name}
                        <li>
                          <button
                            class="row coll"
                            class:active
                            onclick={(e) => openColl(e, conn, db.name, c)}
                            onauxclick={(e) => e.button === 1 && openColl(e, conn, db.name, c)}
                          >
                            <span class="name">{c.name}</span>
                            {#if c.kind !== "collection"}<span class="meta">{c.kind}</span>{/if}
                          </button>
                        </li>
                      {/each}
                    </ul>
                  {/if}
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </li>
    {/each}
  </ul>
</aside>

<style>
  aside {
    background: var(--panel);
    border-right: 1px solid var(--border);
    overflow: auto;
    min-width: 0;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 8px 8px 12px;
    color: var(--muted);
    font-weight: 600;
    position: sticky;
    top: 0;
    background: var(--panel);
    z-index: 1;
  }
  .add {
    padding: 4px 12px 14px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 6px;
  }
  button {
    font: inherit;
    color: inherit;
    background: none;
    border: 0;
    cursor: pointer;
    text-align: left;
  }
  .icon {
    padding: 1px 6px;
    border-radius: 4px;
    color: var(--muted);
    font-size: 14px;
  }
  .icon:hover {
    background: var(--hover);
    color: var(--text);
  }
  .icon.danger {
    color: var(--danger);
    font-size: 11px;
  }
  .tree,
  .tree ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 12px;
    box-sizing: border-box;
  }
  .row:hover {
    background: var(--hover);
  }
  .conn {
    padding: 0 6px 0 0;
  }
  .conn .main {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    padding: 5px 0 5px 12px;
    font-weight: 600;
  }
  .actions {
    display: none;
  }
  .conn:hover .actions,
  .conn:focus-within .actions {
    display: flex;
  }
  .db {
    padding-left: 28px;
  }
  .coll {
    padding-left: 54px;
  }
  .coll.active {
    background: var(--active);
  }
  .caret {
    width: 10px;
    flex: none;
    color: var(--muted);
    transition: transform 0.1s;
  }
  .caret.open {
    transform: rotate(90deg);
  }
  .dot {
    width: 7px;
    height: 7px;
    flex: none;
    border-radius: 50%;
    background: var(--border);
  }
  .dot.on {
    background: var(--accent);
  }
  .dot.busy {
    background: var(--t-date);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: var(--muted);
    font-size: 11px;
  }
  .hint,
  .error {
    margin: 2px 12px 4px 28px;
    font-size: 12px;
  }
  .hint {
    color: var(--muted);
  }
  .hint.deep {
    margin-left: 54px;
  }
  .error {
    color: var(--danger);
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
