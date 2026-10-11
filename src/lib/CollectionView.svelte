<script lang="ts">
  import { api } from "./api";
  import { app, target, type Tab } from "./state.svelte";
  import DocumentCard from "./DocumentCard.svelte";
  import DocumentEditor from "./DocumentEditor.svelte";
  import { smartInput } from "./smartInput";

  let { tab }: { tab: Tab } = $props();

  const PAGE_SIZES = [20, 50, 100];
  const from = $derived(tab.page * tab.pageSize);
  const lastPage = $derived(tab.count == null ? null : Math.max(0, Math.ceil(tab.count / tab.pageSize) - 1));
  const hasNext = $derived(lastPage == null ? tab.docs.length === tab.pageSize : tab.page < lastPage);
  const dirty = $derived(
    tab.input.filter !== tab.applied.filter ||
      tab.input.sort !== tab.applied.sort ||
      tab.input.projection !== tab.applied.projection,
  );

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      app.runQuery(tab);
    }
  }

  function reset() {
    tab.input = { filter: "", sort: "", projection: "" };
    app.runQuery(tab);
  }

  const NEW_DOC = "{\n  \n}";
  let notice = $state<string | null>(null);

  async function insert(text: string) {
    const n = await api.insertDocuments(target(tab), text);
    tab.draft = null;
    notice = `${n} doküman eklendi`;
    setTimeout(() => (notice = null), 2500);
    app.refresh(tab);
  }

  function countText(): string {
    if (tab.count === undefined) return "sayılıyor…";
    if (tab.count === null) return "çok";
    return tab.count.toLocaleString("tr-TR");
  }
</script>

<div class="view">
  <form class="query" onsubmit={(e) => (e.preventDefault(), app.runQuery(tab))}>
    <label class="field filter">
      <span>Filter</span>
      <input bind:value={tab.input.filter} use:smartInput onkeydown={onKeydown} placeholder={"{ alan: 'değer' }"} spellcheck="false" autocomplete="off" />
    </label>
    <label class="field">
      <span>Sort</span>
      <input bind:value={tab.input.sort} use:smartInput onkeydown={onKeydown} placeholder={"{ _id: -1 }"} spellcheck="false" autocomplete="off" />
    </label>
    <label class="field">
      <span>Project</span>
      <input bind:value={tab.input.projection} use:smartInput onkeydown={onKeydown} placeholder={"{ alan: 1 }"} spellcheck="false" autocomplete="off" />
    </label>
    <button type="submit" class="primary" class:dirty>Bul</button>
    <button type="button" onclick={reset}>Sıfırla</button>
  </form>

  <div class="toolbar">
    <span class="range">
      {#if tab.docs.length}
        {(from + 1).toLocaleString("tr-TR")}–{(from + tab.docs.length).toLocaleString("tr-TR")} / {countText()}
      {:else if !tab.loading && !tab.error}
        Doküman yok
      {/if}
    </span>
    {#if tab.loading}<span class="spinner">Yükleniyor…</span>{/if}
    {#if notice}<span class="notice">{notice}</span>{/if}
    <span class="spacer"></span>
    {#if tab.kind !== "view"}
      <button class="add" onclick={() => (tab.draft ??= NEW_DOC)} disabled={tab.draft !== null}>+ Ekle</button>
    {/if}
    <select value={tab.pageSize} onchange={(e) => app.setPageSize(tab, Number(e.currentTarget.value))}>
      {#each PAGE_SIZES as n}<option value={n}>{n} / sayfa</option>{/each}
    </select>
    <button onclick={() => app.goToPage(tab, tab.page - 1)} disabled={tab.page === 0 || tab.loading} title="Önceki sayfa">‹</button>
    <button onclick={() => app.goToPage(tab, tab.page + 1)} disabled={!hasNext || tab.loading} title="Sonraki sayfa">›</button>
    <button onclick={() => app.refresh(tab)} disabled={tab.loading} title="Yenile">↻</button>
  </div>

  {#if tab.error}<p class="error">{tab.error}</p>{/if}

  <div class="docs" class:stale={tab.loading}>
    {#if tab.draft !== null}
      <div class="draft">
        <p class="draft-title">Yeni doküman <span>· birden fazla için dizi yaz: [{"{…}"}, {"{…}"}]</span></p>
        <DocumentEditor
          bind:value={tab.draft}
          onsave={insert}
          oncancel={() => (tab.draft = null)}
          saveLabel="Ekle"
        />
      </div>
    {/if}
    {#each tab.docs as doc, i (from + i)}
      <DocumentCard {doc} {tab} />
    {/each}
  </div>
</div>

<style>
  .view {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .query {
    display: flex;
    gap: 8px;
    align-items: flex-end;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 3px;
    flex: 1;
    min-width: 0;
  }
  .field.filter {
    flex: 3;
  }
  .field span {
    font-size: 11px;
    color: var(--muted);
  }
  input,
  select,
  button {
    font: inherit;
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--panel);
  }
  input {
    font: 12px ui-monospace, SFMono-Regular, Menlo, monospace;
    padding: 6px 8px;
  }
  input:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
  button {
    padding: 5px 10px;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
    font-weight: 600;
  }
  .primary.dirty {
    box-shadow: 0 0 0 2px var(--active);
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 14px;
    border-bottom: 1px solid var(--border);
    color: var(--muted);
  }
  .toolbar button {
    padding: 2px 9px;
  }
  .toolbar select {
    padding: 2px 4px;
  }
  .spacer {
    flex: 1;
  }
  .notice {
    color: var(--accent);
    font-size: 12px;
  }
  .toolbar .add {
    margin-right: 6px;
  }
  .draft {
    padding: 10px;
    border: 1px solid var(--accent);
    border-radius: 6px;
  }
  .draft-title {
    margin: 0 0 8px;
    font-weight: 600;
  }
  .draft-title span {
    font-weight: 400;
    color: var(--muted);
    font-size: 12px;
  }
  .spinner {
    font-size: 11px;
  }
  .docs {
    flex: 1;
    overflow: auto;
    padding: 10px 14px 24px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .docs.stale {
    opacity: 0.6;
  }
  .error {
    margin: 8px 14px 0;
    color: var(--danger);
    white-space: pre-wrap;
    word-break: break-word;
    font: 12px ui-monospace, Menlo, monospace;
  }
</style>
