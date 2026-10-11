<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "$lib/state.svelte";
  import Sidebar from "$lib/Sidebar.svelte";
  import CollectionView from "$lib/CollectionView.svelte";
  import ConnectionForm from "$lib/ConnectionForm.svelte";

  let loaded = $state(false);

  onMount(async () => {
    await app.loadSaved();
    loaded = true;
  });
</script>

<div class="app">
  <Sidebar />
  <main>
    {#if app.tabs.length}
      <nav class="tabs">
        {#each app.tabs as tab (tab.id)}
          <div
            class="tab"
            class:active={tab.id === app.activeTabId}
            title="{tab.connName} › {tab.db}.{tab.coll}"
            role="tab"
            tabindex="0"
            aria-selected={tab.id === app.activeTabId}
            onclick={() => (app.activeTabId = tab.id)}
            onkeydown={(e) => e.key === "Enter" && (app.activeTabId = tab.id)}
            onauxclick={(e) => e.button === 1 && app.closeTab(tab.id)}
          >
            <span class="label">
              <span class="coll">{tab.coll}</span>
              <span class="db">{tab.db}</span>
            </span>
            <button
              class="close"
              onclick={(e) => (e.stopPropagation(), app.closeTab(tab.id))}
              title="Sekmeyi kapat">×</button
            >
          </div>
        {/each}
      </nav>
    {/if}

    <section class="content">
      {#if app.activeTab}
        {#key app.activeTab.id}
          <CollectionView tab={app.activeTab} />
        {/key}
      {:else if loaded && app.conns.length === 0}
        <div class="empty">
          <div class="first">
            <h1>humon</h1>
            <p>İlk bağlantını ekle. Kaydedilir, bir dahaki sefere tek tıkla açılır.</p>
            <ConnectionForm />
          </div>
        </div>
      {:else if loaded}
        <div class="empty">
          <p class="hint">
            Soldan bir bağlantı ve collection seç.<br />
            <kbd>⌘</kbd> + tık ya da orta tık aynı collection'ı yeni bir sekmede açar.
          </p>
        </div>
      {/if}
    </section>
  </main>
</div>

<style>
  :global(:root) {
    --bg: #ffffff;
    --panel: #f6f7f8;
    --border: #e1e4e8;
    --text: #1f2328;
    --muted: #6a737d;
    --accent: #0b7a52;
    --hover: #eaeef2;
    --active: #dff3ea;
    --danger: #c62828;
    --key: #8250df;
    --t-string: #0a3069;
    --t-number: #0550ae;
    --t-keyword: #cf222e;
    --t-id: #116329;
    --t-date: #953800;
    --t-other: #6639ba;
    color-scheme: light;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    font-size: 13px;
  }
  @media (prefers-color-scheme: dark) {
    :global(:root:not([data-theme="light"])) {
      --bg: #16181b;
      --panel: #1d2024;
      --border: #2d3136;
      --text: #e6e8ea;
      --muted: #8b949e;
      --accent: #2fb57c;
      --hover: #272b30;
      --active: #1b3a2d;
      --danger: #ff6b6b;
      --key: #d2a8ff;
      --t-string: #a5d6ff;
      --t-number: #79c0ff;
      --t-keyword: #ff7b72;
      --t-id: #7ee787;
      --t-date: #ffa657;
      --t-other: #d2a8ff;
      color-scheme: dark;
    }
  }
  :global(body) {
    margin: 0;
    background: var(--bg);
    color: var(--text);
    overflow: hidden;
  }
  :global(*) {
    box-sizing: border-box;
  }

  .app {
    display: grid;
    grid-template-columns: 290px 1fr;
    height: 100vh;
  }
  main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .tabs {
    display: flex;
    overflow-x: auto;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
    flex: none;
    scrollbar-width: none;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 220px;
    padding: 6px 6px 6px 12px;
    border-right: 1px solid var(--border);
    cursor: pointer;
    user-select: none;
    color: var(--muted);
  }
  .tab:hover {
    background: var(--hover);
  }
  .tab.active {
    background: var(--bg);
    color: var(--text);
    box-shadow: inset 0 2px 0 var(--accent);
  }
  .label {
    display: flex;
    flex-direction: column;
    min-width: 0;
    line-height: 1.25;
  }
  .label > span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .coll {
    font-weight: 600;
  }
  .db {
    font-size: 10px;
    color: var(--muted);
  }
  .close {
    border: 0;
    background: none;
    color: var(--muted);
    font-size: 14px;
    line-height: 1;
    padding: 2px 5px;
    border-radius: 4px;
    cursor: pointer;
    visibility: hidden;
  }
  .tab:hover .close,
  .tab.active .close {
    visibility: visible;
  }
  .close:hover {
    background: var(--hover);
    color: var(--text);
  }
  .content {
    flex: 1;
    min-height: 0;
  }
  .empty {
    display: grid;
    place-items: center;
    height: 100%;
    padding: 16px;
  }
  .first {
    width: min(480px, 100%);
  }
  .first h1 {
    margin: 0 0 4px;
    font-size: 22px;
  }
  .first p {
    margin: 0 0 18px;
    color: var(--muted);
  }
  .hint {
    color: var(--muted);
    text-align: center;
    line-height: 1.7;
  }
  kbd {
    font: 11px ui-monospace, Menlo, monospace;
    padding: 1px 5px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--panel);
  }
</style>
