<script lang="ts">
  import { classify, preview } from "./ejson";
  import JsonNode from "./JsonNode.svelte";

  let { name, value, depth = 0 }: { name: string; value: unknown; depth?: number } = $props();

  const node = $derived(classify(value));
  const container = $derived(node.kind === "object" || node.kind === "array");
  // Nested values start collapsed so large documents stay cheap to render.
  let open = $state(false);
  const leafText = $derived(
    "text" in node ? (node.kind === "string" ? `"${node.text}"` : node.text) : "",
  );
  // Plain JSON types are obvious from their colour; tag only the BSON-specific ones.
  const showType = $derived(!["string", "double", "int", "bool", "null"].includes(node.kind));
  let wrap = $state(false);

  const children = $derived.by((): [string, unknown][] => {
    if (!open) return [];
    if (node.kind === "object") return node.entries;
    if (node.kind === "array") return node.items.map((v, i) => [String(i), v]);
    return [];
  });
</script>

<div class="row" style:padding-left="{depth * 16 + 4}px">
  {#if container}
    <button class="toggle" onclick={() => (open = !open)}>
      <span class="caret" class:open>▸</span>
      <span class="key">{name}</span><span class="colon">:</span>
      <span class="summary">{preview(node)}</span>
    </button>
  {:else}
    <span class="caret-space"></span>
    <span class="key">{name}</span><span class="colon">:</span>
    <button
      class="value {node.kind}"
      class:wrap
      onclick={() => (wrap = !wrap)}
    >
      {leafText}
    </button>
    {#if showType}<span class="type">{node.kind}</span>{/if}
  {/if}
</div>
{#each children as [k, v] (k)}
  <JsonNode name={k} value={v} depth={depth + 1} />
{/each}

<style>
  .row {
    display: flex;
    align-items: baseline;
    gap: 4px;
    min-height: 20px;
    font: 12px/20px ui-monospace, SFMono-Regular, Menlo, monospace;
    min-width: 0;
  }
  button {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    color: inherit;
    cursor: pointer;
    text-align: left;
  }
  .toggle {
    display: flex;
    gap: 4px;
    align-items: baseline;
    min-width: 0;
  }
  .caret,
  .caret-space {
    display: inline-block;
    width: 10px;
    flex: none;
    color: var(--muted);
    transition: transform 0.1s;
  }
  .caret.open {
    transform: rotate(90deg);
  }
  .key {
    color: var(--key);
    flex: none;
  }
  .colon {
    color: var(--muted);
    margin-right: 2px;
  }
  .summary {
    color: var(--muted);
  }
  .value {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: text;
    user-select: text;
  }
  .value.wrap {
    white-space: pre-wrap;
    word-break: break-all;
  }
  .type {
    flex: none;
    color: var(--muted);
    font-size: 10px;
    opacity: 0.8;
  }
  .string {
    color: var(--t-string);
  }
  .int,
  .long,
  .double,
  .decimal {
    color: var(--t-number);
  }
  .bool,
  .null {
    color: var(--t-keyword);
  }
  .oid,
  .uuid {
    color: var(--t-id);
  }
  .date,
  .timestamp {
    color: var(--t-date);
  }
  .regex,
  .binary,
  .other {
    color: var(--t-other);
  }
</style>
