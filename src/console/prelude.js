// The shell's JavaScript side: mongosh-style types (ObjectId, ISODate, ...)
// and their conversion to and from canonical extended JSON, the form values
// take on their way to and from Rust. Loaded once per engine.
(function () {
  "use strict";

  const INT32_MIN = -2147483648;
  const INT32_MAX = 2147483647;

  // --- base64, which QuickJS doesn't provide -------------------------------
  const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  function bytesToBase64(bytes) {
    let out = "";
    for (let i = 0; i < bytes.length; i += 3) {
      const n = (bytes[i] << 16) | ((bytes[i + 1] || 0) << 8) | (bytes[i + 2] || 0);
      out += B64[(n >> 18) & 63] + B64[(n >> 12) & 63];
      out += i + 1 < bytes.length ? B64[(n >> 6) & 63] : "=";
      out += i + 2 < bytes.length ? B64[n & 63] : "=";
    }
    return out;
  }
  function base64ToBytes(text) {
    const clean = text.replace(/=+$/, "");
    const bytes = [];
    for (let i = 0; i < clean.length; i += 4) {
      const n = [0, 1, 2, 3].reduce((acc, k) => (acc << 6) | Math.max(0, B64.indexOf(clean[i + k] || "A")), 0);
      bytes.push((n >> 16) & 255);
      if (i + 2 < clean.length) bytes.push((n >> 8) & 255);
      if (i + 3 < clean.length) bytes.push(n & 255);
    }
    return bytes;
  }
  const hex = (bytes) => bytes.map((b) => b.toString(16).padStart(2, "0")).join("");

  // --- types: callable with or without `new`, like mongosh ------------------
  function ObjectId(id) {
    if (!(this instanceof ObjectId)) return new ObjectId(id);
    if (id instanceof ObjectId) id = id.$oid;
    if (id === undefined) id = __newObjectId();
    if (typeof id !== "string" || !/^[0-9a-fA-F]{24}$/.test(id)) throw new TypeError("ObjectId needs 24 hex characters");
    this.$oid = id.toLowerCase();
  }
  ObjectId.prototype.toHexString = function () { return this.$oid; };
  ObjectId.prototype.toString = function () { return this.$oid; };
  ObjectId.prototype.equals = function (o) { return o instanceof ObjectId && o.$oid === this.$oid; };
  ObjectId.prototype.getTimestamp = function () { return new Date(parseInt(this.$oid.slice(0, 8), 16) * 1000); };

  function Long(value) {
    if (!(this instanceof Long)) return new Long(value);
    if (value instanceof Long) value = value.value;
    this.value = BigInt(typeof value === "number" ? Math.trunc(value) : String(value).trim()).toString();
  }
  Long.prototype.toString = function () { return this.value; };
  Long.prototype.toNumber = function () { return Number(this.value); };
  Long.prototype.valueOf = function () { return Number(this.value); };

  function Decimal128(value) {
    if (!(this instanceof Decimal128)) return new Decimal128(value);
    this.value = String(value);
  }
  Decimal128.prototype.toString = function () { return this.value; };

  function Binary(base64, subType) {
    if (!(this instanceof Binary)) return new Binary(base64, subType);
    this.base64 = base64;
    this.subType = subType || 0;
  }
  Binary.prototype.toString = function () {
    if (this.subType !== 4) return this.base64;
    const h = hex(base64ToBytes(this.base64));
    return [h.slice(0, 8), h.slice(8, 12), h.slice(12, 16), h.slice(16, 20), h.slice(20)].join("-");
  };

  function UUID(text) {
    const h = String(text).replace(/-/g, "");
    if (!/^[0-9a-fA-F]{32}$/.test(h)) throw new TypeError("UUID needs 32 hex digits");
    const bytes = [];
    for (let i = 0; i < 32; i += 2) bytes.push(parseInt(h.slice(i, i + 2), 16));
    return new Binary(bytesToBase64(bytes), 4);
  }

  function Timestamp(t, i) {
    if (!(this instanceof Timestamp)) return new Timestamp(t, i);
    if (t !== null && typeof t === "object") ({ t, i } = t);
    this.t = t >>> 0;
    this.i = i >>> 0;
  }

  const MIN_KEY = Object.freeze({ toString: () => "MinKey" });
  const MAX_KEY = Object.freeze({ toString: () => "MaxKey" });

  function ISODate(text) {
    return text === undefined ? new Date() : new Date(__isoDate(String(text)));
  }

  Object.assign(globalThis, {
    ObjectId, Long, Decimal128, Binary, UUID, Timestamp, ISODate,
    NumberLong: Long,
    NumberDecimal: Decimal128,
    NumberInt: (v) => parseInt(v, 10),
    BinData: (subType, base64) => new Binary(base64, subType),
    MinKey: () => MIN_KEY,
    MaxKey: () => MAX_KEY,
  });

  // --- extended JSON --------------------------------------------------------
  function toEJSON(v) {
    if (v === null || v === undefined) return null;
    switch (typeof v) {
      case "number":
        return Number.isInteger(v) && v >= INT32_MIN && v <= INT32_MAX && !Object.is(v, -0)
          ? { $numberInt: String(v) }
          : { $numberDouble: String(v) };
      case "string":
      case "boolean":
        return v;
      case "bigint":
        return { $numberLong: v.toString() };
      case "function":
        return "[Function " + (v.name || "anonymous") + "]";
    }
    if (v instanceof ObjectId) return { $oid: v.$oid };
    if (v instanceof Date) return { $date: { $numberLong: String(v.getTime()) } };
    if (v instanceof Long) return { $numberLong: v.value };
    if (v instanceof Decimal128) return { $numberDecimal: v.value };
    if (v instanceof Binary) return { $binary: { base64: v.base64, subType: v.subType.toString(16).padStart(2, "0") } };
    if (v instanceof Timestamp) return { $timestamp: { t: v.t, i: v.i } };
    if (v instanceof RegExp) {
      // BSON knows i, m, s, u, x; JS-only flags like g and y have no meaning there.
      const options = v.flags.split("").filter((f) => "imsux".includes(f)).sort().join("");
      return { $regularExpression: { pattern: v.source, options } };
    }
    if (v === MIN_KEY) return { $minKey: 1 };
    if (v === MAX_KEY) return { $maxKey: 1 };
    if (Array.isArray(v)) return v.map(toEJSON);
    if (typeof v.__ejson === "function") return toEJSON(v.__ejson());
    const out = {};
    for (const k of Object.keys(v)) out[k] = toEJSON(v[k]);
    return out;
  }

  function fromEJSON(v) {
    if (v === null || typeof v !== "object") return v;
    if (Array.isArray(v)) return v.map(fromEJSON);
    const keys = Object.keys(v);
    if (keys.length >= 1 && keys[0][0] === "$") {
      const x = v[keys[0]];
      switch (keys[0]) {
        case "$oid": return new ObjectId(x);
        case "$date": return new Date(typeof x === "object" ? Number(x.$numberLong) : typeof x === "string" ? __isoDate(x) : x);
        case "$numberInt": return parseInt(x, 10);
        case "$numberDouble": return Number(x);
        case "$numberLong": return new Long(x);
        case "$numberDecimal": return new Decimal128(x);
        case "$binary": return new Binary(x.base64, parseInt(x.subType, 16));
        case "$regularExpression": return new RegExp(x.pattern, x.options.split("").filter((f) => "imsu".includes(f)).join(""));
        case "$timestamp": return new Timestamp(x.t, x.i);
        case "$minKey": return MIN_KEY;
        case "$maxKey": return MAX_KEY;
        case "$symbol":
        case "$code": return String(x);
        case "$undefined": return undefined;
      }
    }
    const out = {};
    for (const k of keys) out[k] = fromEJSON(v[k]);
    return out;
  }

  globalThis.__toEJSON = toEJSON;
  globalThis.__fromEJSON = fromEJSON;
  // Strings print as they are; other values are formatted on the Rust side.
  globalThis.print = (...args) => __print(JSON.stringify(args.map(toEJSON)));
})();

// The mongosh-like API. Everything goes through one Rust function,
// __runCommand(db, command), which runs a database command and returns the
// server's reply; collections, cursors and helpers are built on top of it.
(function () {
  "use strict";

  const BATCH = 20;

  function command(dbName, cmd) {
    return __fromEJSON(JSON.parse(__runCommand(dbName, JSON.stringify(__toEJSON(cmd)))));
  }

  // Write commands report ok even when documents failed; surface those.
  function checkWrite(res) {
    if (res.writeErrors && res.writeErrors.length) throw new Error(res.writeErrors[0].errmsg);
    if (res.writeConcernError) throw new Error(res.writeConcernError.errmsg);
    return res;
  }

  const isZero = (id) => id === undefined || id === null || String(id) === "0";

  function Cursor(db, coll, kind, spec) {
    this._db = db;
    this._coll = coll;
    this._kind = kind; // "find" or "aggregate"
    this._spec = spec;
    this._opts = {};
    this._buffer = null;
    this._id = null;
  }
  const option = (name) =>
    function (value) {
      if (this._buffer) throw new Error("cursor already started");
      this._opts[name] = value;
      return this;
    };
  Object.assign(Cursor.prototype, {
    sort: option("sort"),
    limit: option("limit"),
    skip: option("skip"),
    projection: option("projection"),
    project: option("projection"),
    batchSize: option("batchSize"),
    hint: option("hint"),
    maxTimeMS: option("maxTimeMS"),
    pretty() { return this; },
    _start() {
      if (this._buffer) return;
      let cmd;
      if (this._kind === "find") {
        cmd = { find: this._coll, filter: this._spec.filter || {} };
        const projection = this._opts.projection || this._spec.projection;
        if (projection) cmd.projection = projection;
        for (const k of ["sort", "skip", "limit", "batchSize", "hint", "maxTimeMS"]) {
          if (this._opts[k] !== undefined) cmd[k] = this._opts[k];
        }
      } else {
        cmd = { aggregate: this._coll, pipeline: this._spec.pipeline, cursor: {} };
        if (this._opts.batchSize) cmd.cursor.batchSize = this._opts.batchSize;
        Object.assign(cmd, this._spec.options || {});
      }
      const res = command(this._db, cmd);
      this._buffer = res.cursor.firstBatch;
      this._id = res.cursor.id;
    },
    hasNext() {
      this._start();
      while (!this._buffer.length && !isZero(this._id)) {
        const res = command(this._db, { getMore: this._id, collection: this._coll });
        this._buffer = res.cursor.nextBatch;
        this._id = res.cursor.id;
      }
      return this._buffer.length > 0;
    },
    next() {
      if (!this.hasNext()) throw new Error("cursor exhausted");
      return this._buffer.shift();
    },
    toArray() {
      const all = [];
      while (this.hasNext()) all.push(this.next());
      return all;
    },
    forEach(fn) {
      while (this.hasNext()) fn(this.next());
    },
    map(fn) { return this.toArray().map(fn); },
    itcount() { return this.toArray().length; },
    // Up to n documents, for showing one page at a time.
    _take(n) {
      const page = [];
      while (page.length < n && this.hasNext()) page.push(this.next());
      return page;
    },
    close() {
      if (this._buffer && !isZero(this._id)) command(this._db, { killCursors: this._coll, cursors: [this._id] });
      this._id = null;
      this._buffer = [];
    },
  });

  function Collection(db, name) {
    this._db = db;
    this._name = name;
  }
  const withId = (doc) => (doc._id === undefined ? Object.assign({ _id: ObjectId() }, doc) : doc);
  const updateResult = (res) => {
    const upserted = res.upserted || [];
    const out = { acknowledged: true, matchedCount: res.n - upserted.length, modifiedCount: res.nModified, upsertedCount: upserted.length };
    if (upserted.length) out.upsertedId = upserted[0]._id;
    return out;
  };
  Object.assign(Collection.prototype, {
    getName() { return this._name; },
    getFullName() { return this._db + "." + this._name; },
    _cmd(cmd) { return command(this._db, cmd); },
    find(filter, projection) { return new Cursor(this._db, this._name, "find", { filter, projection }); },
    findOne(filter, projection) {
      const cursor = this.find(filter, projection).limit(1);
      return cursor.hasNext() ? cursor.next() : null;
    },
    aggregate(pipeline, options) { return new Cursor(this._db, this._name, "aggregate", { pipeline: pipeline || [], options }); },
    countDocuments(filter) {
      const res = this.aggregate([{ $match: filter || {} }, { $group: { _id: 1, n: { $sum: 1 } } }]).toArray();
      return res.length ? res[0].n : 0;
    },
    estimatedDocumentCount() { return this._cmd({ count: this._name }).n; },
    distinct(key, filter) { return this._cmd({ distinct: this._name, key, query: filter || {} }).values; },
    insertOne(doc) {
      doc = withId(doc);
      checkWrite(this._cmd({ insert: this._name, documents: [doc] }));
      return { acknowledged: true, insertedId: doc._id };
    },
    insertMany(docs) {
      docs = docs.map(withId);
      checkWrite(this._cmd({ insert: this._name, documents: docs, ordered: true }));
      const insertedIds = {};
      docs.forEach((d, i) => (insertedIds[i] = d._id));
      return { acknowledged: true, insertedIds };
    },
    _update(filter, update, options, multi) {
      const u = Object.assign({ q: filter, u: update, multi, upsert: !!(options && options.upsert) }, options && options.arrayFilters ? { arrayFilters: options.arrayFilters } : {});
      return updateResult(checkWrite(this._cmd({ update: this._name, updates: [u] })));
    },
    updateOne(filter, update, options) { return this._update(filter, update, options, false); },
    updateMany(filter, update, options) { return this._update(filter, update, options, true); },
    replaceOne(filter, doc, options) { return this._update(filter, doc, options, false); },
    _delete(filter, limit) {
      const res = checkWrite(this._cmd({ delete: this._name, deletes: [{ q: filter, limit }] }));
      return { acknowledged: true, deletedCount: res.n };
    },
    deleteOne(filter) { return this._delete(filter, 1); },
    deleteMany(filter) { return this._delete(filter, 0); },
    _findAndModify(filter, extra) {
      return this._cmd(Object.assign({ findAndModify: this._name, query: filter }, extra)).value;
    },
    findOneAndUpdate(filter, update, options) {
      const o = options || {};
      return this._findAndModify(filter, { update, new: o.returnDocument === "after" || !!o.returnNewDocument, upsert: !!o.upsert, sort: o.sort, fields: o.projection });
    },
    findOneAndReplace(filter, doc, options) { return this.findOneAndUpdate(filter, doc, options); },
    findOneAndDelete(filter, options) {
      const o = options || {};
      return this._findAndModify(filter, { remove: true, sort: o.sort, fields: o.projection });
    },
    createIndex(keys, options) {
      const name = (options && options.name) || Object.keys(keys).map((k) => k + "_" + keys[k]).join("_");
      this._cmd({ createIndexes: this._name, indexes: [Object.assign({ key: keys, name }, options || {})] });
      return name;
    },
    getIndexes() { return new Cursor(this._db, this._name, "listIndexes", {})._list(); },
    dropIndex(index) { return this._cmd({ dropIndexes: this._name, index }); },
    drop() {
      try {
        this._cmd({ drop: this._name });
        return true;
      } catch (e) {
        if (/ns not found/i.test(String(e.message))) return false;
        throw e;
      }
    },
    renameCollection(to) {
      return command("admin", { renameCollection: this._db + "." + this._name, to: this._db + "." + to });
    },
  });
  // listIndexes answers with a cursor too.
  Cursor.prototype._list = function () {
    const res = command(this._db, { listIndexes: this._coll });
    this._buffer = res.cursor.firstBatch;
    this._id = res.cursor.id;
    return this.toArray();
  };

  function Database(name) {
    this._name = name;
  }
  Object.assign(Database.prototype, {
    getName() { return this._name; },
    getSiblingDB(name) { return dbProxy(name); },
    getCollection(name) { return new Collection(this._name, name); },
    runCommand(cmd) { return command(this._name, cmd); },
    adminCommand(cmd) { return command("admin", cmd); },
    getCollectionInfos(filter) {
      return command(this._name, { listCollections: 1, filter: filter || {} }).cursor.firstBatch;
    },
    getCollectionNames() { return this.getCollectionInfos().map((c) => c.name).sort(); },
    createCollection(name, options) { return command(this._name, Object.assign({ create: name }, options || {})); },
    dropDatabase() { return command(this._name, { dropDatabase: 1 }); },
    stats() { return command(this._name, { dbStats: 1 }); },
    version() { return command("admin", { buildInfo: 1 }).version; },
    serverStatus() { return command("admin", { serverStatus: 1 }); },
  });

  // `db.users` is the users collection, unless `users` is a Database method.
  function dbProxy(name) {
    return new Proxy(new Database(name), {
      get(target, prop) {
        if (typeof prop === "symbol" || prop in target) return target[prop];
        return target.getCollection(prop);
      },
    });
  }
  Object.defineProperty(globalThis, "db", { get: () => dbProxy(globalThis.__db), configurable: true });

  function humanSize(bytes) {
    const units = ["B", "KB", "MB", "GB", "TB"];
    let v = bytes;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i++;
    }
    return (i === 0 ? String(v) : v.toFixed(v < 10 ? 1 : 0)) + " " + units[i];
  }

  globalThis.use = (name) => {
    globalThis.__db = String(name);
    __info("switched", String(name));
  };
  globalThis.show = (what) => {
    if (what === "dbs") {
      const dbs = command("admin", { listDatabases: 1 }).databases;
      const width = Math.max(...dbs.map((d) => d.name.length));
      dbs.forEach((d) => print(d.name.padEnd(width) + "  " + humanSize(Number(d.sizeOnDisk))));
    } else if (what === "collections") {
      db.getCollectionNames().forEach((n) => print(n));
    } else {
      throw new Error("show: unknown option " + what);
    }
  };

  // The cursor whose next page `it` shows.
  let lastCursor = null;
  globalThis.it = () => {
    if (!lastCursor) throw new Error(__tr("no_cursor"));
    return lastCursor;
  };

  // A cursor shows one page and remembers itself for `it`; anything else
  // shows as it is. Returns {v, more} as JSON, or "" for nothing to show.
  globalThis.__display = (v) => {
    if (v === undefined) return "";
    if (v instanceof Cursor) {
      if (lastCursor && lastCursor !== v) lastCursor.close();
      lastCursor = v;
      const page = v._take(BATCH);
      return JSON.stringify({ v: __toEJSON(page), more: v.hasNext() });
    }
    return JSON.stringify({ v: __toEJSON(v), more: false });
  };
})();
