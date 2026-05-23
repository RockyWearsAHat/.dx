import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const nativeBindingPath = process.env.DOC_SQLITE_NATIVE_BINDING_PATH || '../../../build/Release/doc_sqlite.node';

let nativeBinding = null;
let usingNativeBridge = false;

try {
  nativeBinding = require(nativeBindingPath);
  usingNativeBridge = true;
} catch {
  nativeBinding = null;
}

let DatabaseSyncImpl = null;

if (nativeBinding?.DatabaseSync) {
  DatabaseSyncImpl = nativeBinding.DatabaseSync;
} else {
  const sqlite = await import('node:sqlite');
  DatabaseSyncImpl = sqlite.DatabaseSync;
}

export const DatabaseSync = DatabaseSyncImpl;
export const isNativeSQLiteBridge = usingNativeBridge;
