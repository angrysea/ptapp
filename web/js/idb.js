// IndexedDB: a key/value store for cached reads and drafts, an outbox of
// writes waiting for connectivity (LG-3), and set videos waiting to upload
// (VS-3, uploads.js).

const open = () => new Promise((resolve, reject) => {
  const req = indexedDB.open('ptapp', 2);
  req.onupgradeneeded = () => {
    const d = req.result;
    if (!d.objectStoreNames.contains('kv')) d.createObjectStore('kv');
    if (!d.objectStoreNames.contains('outbox')) d.createObjectStore('outbox', { keyPath: 'key' });
    if (!d.objectStoreNames.contains('uploads')) d.createObjectStore('uploads', { keyPath: 'key' });
  };
  req.onsuccess = () => resolve(req.result);
  req.onerror = () => reject(req.error);
});
let dbp;
const db = () => (dbp ||= open());

async function run(store, mode, fn) {
  const d = await db();
  return new Promise((resolve, reject) => {
    const tx = d.transaction(store, mode);
    const req = fn(tx.objectStore(store));
    tx.oncomplete = () => resolve(req?.result);
    tx.onerror = () => reject(tx.error);
  });
}

export const kv = {
  get: (k) => run('kv', 'readonly', (s) => s.get(k)),
  set: (k, v) => run('kv', 'readwrite', (s) => s.put(v, k)),
  del: (k) => run('kv', 'readwrite', (s) => s.delete(k)),
  keys: () => run('kv', 'readonly', (s) => s.getAllKeys()),
  clear: () => run('kv', 'readwrite', (s) => s.clear()),
};

// Strictly increasing, so two writes queued in the same millisecond still get
// distinct seqs (flush() relies on seq to tell a re-queued item from the sent one).
let lastSeq = 0;
const nextSeq = () => (lastSeq = Math.max(Date.now(), lastSeq + 1));

export const outbox = {
  /** Items with the same key replace each other, so only the latest edit is sent. */
  put: (item) => run('outbox', 'readwrite', (s) => s.put({ seq: nextSeq(), ...item })),
  all: async () => ((await run('outbox', 'readonly', (s) => s.getAll())) || []).sort((a, b) => a.seq - b.seq),
  get: (key) => run('outbox', 'readonly', (s) => s.get(key)),
  del: (key) => run('outbox', 'readwrite', (s) => s.delete(key)),
  /** Deletes the item only if it is still the one with `seq`, in one
   * transaction: a newer edit queued under the same key while the old one
   * was being sent is never deleted with it. */
  delIfSeq: (key, seq) => run('outbox', 'readwrite', (s) => {
    const req = s.get(key);
    req.onsuccess = () => { if (req.result?.seq === seq) s.delete(key); };
    return req;
  }),
  clear: () => run('outbox', 'readwrite', (s) => s.clear()),
};

/** Set videos waiting to upload, with the parts already sent (uploads.js). */
export const uploads = {
  put: (item) => run('uploads', 'readwrite', (s) => s.put(item)),
  get: (key) => run('uploads', 'readonly', (s) => s.get(key)),
  all: async () => ((await run('uploads', 'readonly', (s) => s.getAll())) || []).sort((a, b) => a.createdAt.localeCompare(b.createdAt)),
  del: (key) => run('uploads', 'readwrite', (s) => s.delete(key)),
};
