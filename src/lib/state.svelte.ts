import {
  api,
  type CollectionInfo,
  type DatabaseInfo,
  type EJson,
  type SavedConnection,
  type Target,
} from "./api";

export type DbNode = DatabaseInfo & {
  open: boolean;
  loading: boolean;
  collections: CollectionInfo[] | null;
};

export type ConnNode = {
  saved: SavedConnection;
  connId: string | null;
  connecting: boolean;
  open: boolean;
  error: string | null;
  dbs: DbNode[];
};

type Query = { filter: string; sort: string; projection: string };

export type Tab = {
  id: number;
  connId: string;
  connName: string;
  db: string;
  coll: string;
  kind: CollectionInfo["kind"];
  /** Text of the insert editor; null when it is closed. */
  draft: string | null;
  /** What is typed in the query bar. */
  input: Query;
  /** What the shown results were fetched with; paging reuses it. */
  applied: Query;
  page: number;
  pageSize: number;
  docs: EJson[];
  /** undefined while counting, null when the count was too slow to finish. */
  count: number | null | undefined;
  loading: boolean;
  error: string | null;
  /** Bumped per request so a slow, stale response can't overwrite a newer one. */
  seq: number;
  countSeq: number;
};

const emptyQuery = (): Query => ({ filter: "", sort: "", projection: "" });

export const target = (tab: Tab): Target => ({ id: tab.connId, db: tab.db, coll: tab.coll });

function message(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

class AppState {
  conns = $state<ConnNode[]>([]);
  tabs = $state<Tab[]>([]);
  activeTabId = $state<number | null>(null);
  activeTab = $derived(this.tabs.find((t) => t.id === this.activeTabId) ?? null);
  loadError = $state<string | null>(null);
  #nextTabId = 1;

  async loadSaved() {
    try {
      const list = await api.listSaved();
      this.conns = list.map((saved) => ({
        saved,
        connId: null,
        connecting: false,
        open: false,
        error: null,
        dbs: [],
      }));
    } catch (e) {
      this.loadError = message(e);
    }
  }

  /** Saves a connection and connects to it. Throws so the form can show the error. */
  async addConnection(name: string, uri: string) {
    const saved = await api.saveConnection(name, uri);
    this.conns.push({ saved, connId: null, connecting: false, open: false, error: null, dbs: [] });
    // Read back through the proxy so later mutations stay reactive.
    await this.connect(this.conns[this.conns.length - 1]);
  }

  async connect(conn: ConnNode) {
    conn.connecting = true;
    conn.error = null;
    try {
      conn.connId = await api.connectSaved(conn.saved.id);
      await this.refreshDatabases(conn);
      conn.open = true;
    } catch (e) {
      conn.error = message(e);
      if (conn.connId) await this.disconnect(conn);
    } finally {
      conn.connecting = false;
    }
  }

  async refreshDatabases(conn: ConnNode) {
    if (!conn.connId) return;
    const list = await api.listDatabases(conn.connId);
    const wasOpen = new Set(conn.dbs.filter((d) => d.open).map((d) => d.name));
    conn.dbs = list.map((d) => ({ ...d, open: false, loading: false, collections: null }));
    for (const db of conn.dbs) if (wasOpen.has(db.name)) await this.toggleDatabase(conn, db);
  }

  async toggleConnection(conn: ConnNode) {
    if (conn.connecting) return;
    if (!conn.connId) await this.connect(conn);
    else conn.open = !conn.open;
  }

  async disconnect(conn: ConnNode) {
    const id = conn.connId;
    if (!id) return;
    for (const tab of this.tabs.filter((t) => t.connId === id)) this.closeTab(tab.id);
    conn.connId = null;
    conn.open = false;
    conn.dbs = [];
    await api.disconnect(id).catch(() => {});
  }

  async removeConnection(conn: ConnNode) {
    await this.disconnect(conn);
    await api.deleteSaved(conn.saved.id);
    this.conns = this.conns.filter((c) => c.saved.id !== conn.saved.id);
  }

  async toggleDatabase(conn: ConnNode, db: DbNode) {
    db.open = !db.open;
    if (!db.open || db.collections || !conn.connId) return;
    db.loading = true;
    try {
      db.collections = await api.listCollections(conn.connId, db.name);
    } catch (e) {
      conn.error = message(e);
      db.open = false;
    } finally {
      db.loading = false;
    }
  }

  /** Focuses the collection's tab, or opens one. `newTab` always opens another. */
  openCollection(conn: ConnNode, db: string, coll: CollectionInfo, newTab = false) {
    if (!conn.connId) return;
    const existing = newTab
      ? undefined
      : this.tabs.find((t) => t.connId === conn.connId && t.db === db && t.coll === coll.name);
    if (existing) {
      this.activeTabId = existing.id;
      return;
    }
    this.tabs.push({
      id: this.#nextTabId++,
      connId: conn.connId,
      connName: conn.saved.name,
      db,
      coll: coll.name,
      kind: coll.kind,
      draft: null,
      input: emptyQuery(),
      applied: emptyQuery(),
      page: 0,
      pageSize: 20,
      docs: [],
      count: undefined,
      loading: false,
      error: null,
      seq: 0,
      countSeq: 0,
    });
    const tab = this.tabs[this.tabs.length - 1];
    this.activeTabId = tab.id;
    this.runQuery(tab);
  }

  closeTab(id: number) {
    const index = this.tabs.findIndex((t) => t.id === id);
    if (index < 0) return;
    this.tabs.splice(index, 1);
    if (this.activeTabId === id) {
      this.activeTabId = (this.tabs[index] ?? this.tabs[index - 1])?.id ?? null;
    }
  }

  /** Applies what is typed in the query bar and goes back to the first page. */
  runQuery(tab: Tab) {
    tab.applied = { ...tab.input };
    tab.page = 0;
    this.recount(tab);
    return this.loadPage(tab, ++tab.seq);
  }

  /** Reloads the current page and count with the applied query, e.g. after a write. */
  refresh(tab: Tab) {
    this.recount(tab);
    return this.loadPage(tab, ++tab.seq);
  }

  private recount(tab: Tab) {
    tab.count = undefined;
    const countSeq = ++tab.countSeq;
    api
      .countDocuments({ ...target(tab), filter: tab.applied.filter })
      .then((n) => {
        if (tab.countSeq === countSeq) tab.count = n;
      })
      .catch(() => {
        if (tab.countSeq === countSeq) tab.count = null;
      });
  }

  goToPage(tab: Tab, page: number): Promise<void> {
    tab.page = Math.max(0, page);
    return this.loadPage(tab, ++tab.seq);
  }

  setPageSize(tab: Tab, size: number) {
    tab.pageSize = size;
    return this.goToPage(tab, 0);
  }

  private async loadPage(tab: Tab, seq: number): Promise<void> {
    tab.loading = true;
    tab.error = null;
    try {
      const docs = await api.findDocuments(
        target(tab),
        { ...tab.applied, skip: tab.page * tab.pageSize, limit: tab.pageSize },
      );
      if (tab.seq !== seq) return;
      // A delete can empty the last page; step back instead of showing nothing.
      if (docs.length === 0 && tab.page > 0) return this.goToPage(tab, tab.page - 1);
      tab.docs = docs;
    } catch (e) {
      if (tab.seq === seq) {
        tab.error = message(e);
        tab.docs = [];
      }
    } finally {
      if (tab.seq === seq) tab.loading = false;
    }
  }
}

export const app = new AppState();
