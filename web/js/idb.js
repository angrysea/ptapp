// IndexedDB: a key/value store for cached reads and drafts, and an outbox of
// writes waiting for connectivity (LG-3).

const open = () => new Promise((resolve, reject) => {
  const req = indexedDB.open('ptapp', 1);
  req.onupgradeneeded = () => {
    req.result.createObjectStore('kv');
    req.result.createObjectStore('outbox', { keyPath: 'key' });
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

export const outbox = {
  /** Items with the same key replace each other, so only the latest edit is sent. */
  put: (item) => run('outbox', 'readwrite', (s) => s.put({ seq: Date.now(), ...item })),
  all: async () => ((await run('outbox', 'readonly', (s) => s.getAll())) || []).sort((a, b) => a.seq - b.seq),
  del: (key) => run('outbox', 'readwrite', (s) => s.delete(key)),
  clear: () => run('outbox', 'readwrite', (s) => s.clear()),
};
