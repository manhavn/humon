import { invoke } from "@tauri-apps/api/core";

export type SavedConnection = { id: string; name: string; uri: string };
export type DatabaseInfo = { name: string; sizeOnDisk: number };
export type CollectionInfo = { name: string; kind: "collection" | "view" | "timeseries" };
/** A document in canonical extended JSON. */
export type EJson = unknown;
/** Which collection a document call is about. */
export type Target = { id: string; db: string; coll: string };

export const api = {
  listSaved: () => invoke<SavedConnection[]>("list_saved"),
  saveConnection: (name: string, uri: string, id?: string) =>
    invoke<SavedConnection>("save_connection", { id: id ?? null, name, uri }),
  deleteSaved: (id: string) => invoke<void>("delete_saved", { id }),

  connect: (uri: string) => invoke<string>("connect", { uri }),
  connectSaved: (savedId: string) => invoke<string>("connect_saved", { savedId }),
  disconnect: (id: string) => invoke<void>("disconnect", { id }),

  listDatabases: (id: string) => invoke<DatabaseInfo[]>("list_databases", { id }),
  listCollections: (id: string, db: string) =>
    invoke<CollectionInfo[]>("list_collections", { id, db }),

  findDocuments: (
    target: Target,
    args: { filter: string; sort: string; projection: string; skip: number; limit: number },
  ) => invoke<EJson[]>("find_documents", { ...target, args }),
  countDocuments: (args: { id: string; db: string; coll: string; filter: string }) =>
    invoke<number | null>("count_documents", args),

  getDocument: (target: Target, docId: EJson) =>
    invoke<EJson | null>("get_document", { ...target, docId }),
  /** `doc` is shell syntax; returns the stored document. */
  replaceDocument: (target: Target, docId: EJson, doc: string) =>
    invoke<EJson>("replace_document", { ...target, docId, doc }),
  deleteDocument: (target: Target, docId: EJson) =>
    invoke<void>("delete_document", { ...target, docId }),
  /** One document or an array, in shell syntax; returns how many were inserted. */
  insertDocuments: (target: Target, docs: string) =>
    invoke<number>("insert_documents", { ...target, docs }),
};
