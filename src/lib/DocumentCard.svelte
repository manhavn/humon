<script lang="ts">
  import { api } from "./api";
  import { classify, toShell } from "./ejson";
  import { app, target, type Tab } from "./state.svelte";
  import DocumentEditor from "./DocumentEditor.svelte";
  import JsonNode from "./JsonNode.svelte";

  let { doc, tab }: { doc: unknown; tab: Tab } = $props();

  const node = $derived(classify(doc));
  const entries = $derived(node.kind === "object" ? node.entries : []);
  const docId = $derived(entries.find(([k]) => k === "_id")?.[1]);
  // Views are read-only, and without _id (e.g. projected away) there is nothing to target.
  const writable = $derived(tab.kind !== "view" && docId !== undefined);

  let copyStatus = $state<"idle" | "copied" | "failed">("idle");
  let editText = $state<string | null>(null);
  let busy = $state(false);
  let confirmDelete = $state(false);
  let error = $state<string | null>(null);

  async function copy() {
    try {
      await navigator.clipboard.writeText(toShell(doc));
      copyStatus = "copied";
    } catch {
      copyStatus = "failed";
    }
    setTimeout(() => (copyStatus = "idle"), 1200);
  }

  async function startEdit() {
    busy = true;
    error = null;
    try {
      // Edit the full stored document: the tab may show a projection, and
      // saving a projected copy would drop the hidden fields.
      const full = await api.getDocument(target(tab), docId);
      if (full === null) throw "doküman bulunamadı; başka biri silmiş olabilir";
      editText = toShell(full);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function save(text: string) {
    await api.replaceDocument(target(tab), docId, text);
    editText = null;
    app.refresh(tab);
  }

  function clone() {
    tab.draft = toShell(Object.fromEntries(entries.filter(([k]) => k !== "_id")));
  }

  async function remove() {
    if (!confirmDelete) {
      confirmDelete = true;
      setTimeout(() => (confirmDelete = false), 3000);
      return;
    }
    confirmDelete = false;
    busy = true;
    error = null;
    try {
      await api.deleteDocument(target(tab), docId);
      app.refresh(tab);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<article class:editing={editText !== null}>
  {#if editText !== null}
    <DocumentEditor bind:value={editText} onsave={save} oncancel={() => (editText = null)} />
  {:else}
    <div class="actions">
      {#if writable}
        <button onclick={startEdit} disabled={busy}>Düzenle</button>
        <button onclick={clone} disabled={busy}>Klonla</button>
      {/if}
      <button onclick={copy} title="Shell formatında kopyala">
        {copyStatus === "copied" ? "Kopyalandı" : copyStatus === "failed" ? "Kopyalanamadı" : "Kopyala"}
      </button>
      {#if writable}
        <button class="danger" class:confirm={confirmDelete} onclick={remove} disabled={busy}>
          {confirmDelete ? "Emin misin?" : "Sil"}
        </button>
      {/if}
    </div>
    {#each entries as [k, v] (k)}
      <JsonNode name={k} value={v} />
    {/each}
  {/if}
  {#if error}<p class="error">{error}</p>{/if}
</article>

<style>
  article {
    position: relative;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
  }
  article.editing {
    border-color: var(--accent);
  }
  .actions {
    position: absolute;
    top: 6px;
    right: 8px;
    display: flex;
    gap: 4px;
    opacity: 0;
  }
  article:hover .actions,
  .actions:focus-within {
    opacity: 1;
  }
  .actions button {
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--panel);
    color: var(--muted);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }
  .actions button:hover {
    color: var(--text);
  }
  .actions button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .danger:hover,
  .danger.confirm {
    color: var(--danger) !important;
    border-color: var(--danger) !important;
  }
  .danger.confirm {
    opacity: 1;
  }
  .error {
    margin: 6px 0 0;
    color: var(--danger);
    white-space: pre-wrap;
    word-break: break-word;
    font: 12px ui-monospace, Menlo, monospace;
  }
</style>
