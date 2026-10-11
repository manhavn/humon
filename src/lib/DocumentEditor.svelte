<script lang="ts">
  import { onMount } from "svelte";
  import { insertText, smartInput } from "./smartInput";

  let {
    value = $bindable(),
    onsave,
    oncancel,
    saveLabel = "Kaydet",
  }: {
    value: string;
    /** Throws to keep the editor open and show the error. */
    onsave: (text: string) => Promise<void>;
    oncancel: () => void;
    saveLabel?: string;
  } = $props();

  let saving = $state(false);
  let error = $state<string | null>(null);
  let textarea: HTMLTextAreaElement;
  const rows = $derived(Math.min(30, Math.max(6, value.split("\n").length + 1)));

  onMount(() => textarea.focus());

  async function save() {
    if (saving) return;
    saving = true;
    error = null;
    try {
      await onsave(value);
    } catch (e) {
      error = String(e);
    } finally {
      saving = false;
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      save();
    } else if (e.key === "Escape") {
      e.preventDefault();
      oncancel();
    } else if (e.key === "Tab" && !e.shiftKey) {
      // Indent instead of leaving the field.
      e.preventDefault();
      insertText(textarea, "  ");
    }
  }
</script>

<div class="editor">
  <textarea bind:this={textarea} bind:value {rows} use:smartInput onkeydown={onKeydown} spellcheck="false"></textarea>
  {#if error}<p class="error">{error}</p>{/if}
  <div class="actions">
    <span class="hint"><kbd>⌘</kbd><kbd>↵</kbd> kaydet · <kbd>esc</kbd> vazgeç</span>
    <button onclick={oncancel} disabled={saving}>Vazgeç</button>
    <button class="primary" onclick={save} disabled={saving}>{saving ? "Kaydediliyor…" : saveLabel}</button>
  </div>
</div>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  textarea {
    width: 100%;
    resize: vertical;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--panel);
    color: var(--text);
    font: 12px/1.55 ui-monospace, SFMono-Regular, Menlo, monospace;
    tab-size: 2;
  }
  textarea:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .hint {
    flex: 1;
    color: var(--muted);
    font-size: 11px;
  }
  kbd {
    font: 10px ui-monospace, Menlo, monospace;
    padding: 0 4px;
    margin-right: 2px;
    border: 1px solid var(--border);
    border-radius: 3px;
  }
  button {
    font: inherit;
    padding: 4px 12px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--panel);
    color: var(--text);
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
    font-weight: 600;
  }
  .error {
    margin: 0;
    color: var(--danger);
    white-space: pre-wrap;
    word-break: break-word;
    font: 12px ui-monospace, Menlo, monospace;
  }
</style>
