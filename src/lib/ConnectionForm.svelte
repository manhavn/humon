<script lang="ts">
  import { app } from "./state.svelte";

  let { oncancel }: { oncancel?: () => void } = $props();

  let name = $state("");
  let uri = $state("");
  let saving = $state(false);
  let error = $state<string | null>(null);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    saving = true;
    error = null;
    try {
      await app.addConnection(name, uri);
      const added = app.conns[app.conns.length - 1];
      // Saved either way; a failed connect shows its error on the sidebar row.
      if (added?.error) error = added.error;
      else oncancel?.();
    } catch (err) {
      error = String(err);
    } finally {
      saving = false;
    }
  }
</script>

<form onsubmit={submit}>
  <label>
    <span>Ad</span>
    <input bind:value={name} placeholder="Örn. Prod Atlas" autocomplete="off" />
  </label>
  <label>
    <span>Bağlantı adresi</span>
    <input
      class="mono"
      bind:value={uri}
      placeholder="mongodb+srv://kullanici:sifre@cluster.mongodb.net"
      spellcheck="false"
      autocomplete="off"
    />
  </label>
  <div class="actions">
    {#if oncancel}<button type="button" onclick={oncancel}>İptal</button>{/if}
    <button type="submit" class="primary" disabled={saving || !uri.trim()}>
      {saving ? "Bağlanıyor…" : "Kaydet ve bağlan"}
    </button>
  </div>
  {#if error}<p class="error">{error}</p>{/if}
</form>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  label span {
    color: var(--muted);
    font-size: 12px;
  }
  input {
    font: inherit;
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--text);
  }
  .mono {
    font: 12px ui-monospace, SFMono-Regular, Menlo, monospace;
  }
  input:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  button {
    font: inherit;
    padding: 6px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--panel);
    color: var(--text);
    cursor: pointer;
  }
  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
    font-weight: 600;
  }
  button:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .error {
    margin: 0;
    color: var(--danger);
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 12px;
  }
</style>
