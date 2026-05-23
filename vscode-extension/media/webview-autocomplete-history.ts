const AUTOCOMPLETE_HISTORY_MAX_ITEMS = 300;

interface AutocompleteSchema {
  blockTypes: string[];
  attributeKeys: string[];
  attributeValuesByKey: Record<string, string[]>;
}

type TokenKind = 'block' | 'attribute-key' | 'attribute-value';

interface StorageLike {
  getItem: (key: string) => string | null;
  setItem: (key: string, value: string) => void;
}

function addUniqueItems(target: string[], values: Array<string | number | boolean | null | undefined>): void {
  const seen = new Set(target);
  for (const value of values || []) {
    const text = String(value || '').trim();
    if (!text || seen.has(text)) continue;
    target.push(text);
    seen.add(text);
  }
}

function normalizeHistory(raw: string | number | boolean | null | undefined | object): AutocompleteSchema {
  const history: AutocompleteSchema = {
    blockTypes: [],
    attributeKeys: [],
    attributeValuesByKey: {},
  };

  if (!raw || typeof raw !== 'object') {
    return history;
  }

  const source = raw as Record<string, string | number | boolean | null | undefined | object | string[] | Record<string, string[]>>;

  history.blockTypes = Array.isArray(source.blockTypes)
    ? source.blockTypes.map((value) => String(value || '').trim()).filter(Boolean)
    : [];
  history.attributeKeys = Array.isArray(source.attributeKeys)
    ? source.attributeKeys.map((value) => String(value || '').trim()).filter(Boolean)
    : [];

  if (source.attributeValuesByKey && typeof source.attributeValuesByKey === 'object') {
    for (const [key, values] of Object.entries(source.attributeValuesByKey)) {
      if (!Array.isArray(values)) continue;
      history.attributeValuesByKey[String(key).toLowerCase()] = values
        .map((value) => String(value || '').trim())
        .filter(Boolean);
    }
  }

  return history;
}

function createEmptySchema(): AutocompleteSchema {
  return {
    blockTypes: [],
    attributeKeys: [],
    attributeValuesByKey: {},
  };
}

export function buildAutocompleteSchemaFromHeaders(headers: string[]): AutocompleteSchema {
  const schema = createEmptySchema();
  const blockTypeSet = new Set<string>();
  const attributeKeySet = new Set<string>();

  for (const headerLine of headers || []) {
    const line = String(headerLine || '').trim();
    const header = /^::([a-z-]+)(?:\s+(.*))?$/i.exec(line);
    if (!header) continue;

    const blockType = String(header[1] || '').toLowerCase();
    if (blockType && !blockTypeSet.has(blockType)) {
      blockTypeSet.add(blockType);
      schema.blockTypes.push(blockType);
    }

    const attrsText = String(header[2] || '');
    const attrPattern = /([a-zA-Z0-9._-]+)=(?:"([^"]*)"|'([^']*)'|([^\s]+))/g;
    let match = attrPattern.exec(attrsText);
    while (match) {
      const key = String(match[1] || '').toLowerCase();
      const value = String(match[2] ?? match[3] ?? match[4] ?? '').trim();

      if (key && !attributeKeySet.has(key)) {
        attributeKeySet.add(key);
        schema.attributeKeys.push(key);
      }

      if (key && value) {
        if (!Array.isArray(schema.attributeValuesByKey[key])) {
          schema.attributeValuesByKey[key] = [];
        }
        if (!schema.attributeValuesByKey[key].includes(value)) {
          schema.attributeValuesByKey[key].push(value);
        }
      }

      match = attrPattern.exec(attrsText);
    }
  }

  return schema;
}

export function mergeAutocompleteSchemas(primary: AutocompleteSchema, secondary: AutocompleteSchema): AutocompleteSchema {
  const merged = createEmptySchema();

  addUniqueItems(merged.blockTypes, primary.blockTypes);
  addUniqueItems(merged.blockTypes, secondary.blockTypes);
  addUniqueItems(merged.attributeKeys, primary.attributeKeys);
  addUniqueItems(merged.attributeKeys, secondary.attributeKeys);

  for (const [key, values] of Object.entries(primary.attributeValuesByKey || {})) {
    if (!Array.isArray(merged.attributeValuesByKey[key])) {
      merged.attributeValuesByKey[key] = [];
    }
    addUniqueItems(merged.attributeValuesByKey[key], values);
  }

  for (const [key, values] of Object.entries(secondary.attributeValuesByKey || {})) {
    if (!Array.isArray(merged.attributeValuesByKey[key])) {
      merged.attributeValuesByKey[key] = [];
    }
    addUniqueItems(merged.attributeValuesByKey[key], values);
  }

  return merged;
}

export function createAutocompleteHistory(storage: StorageLike | null | undefined, storageKey: string) {
  const canStore = storage
    && typeof storage.getItem === 'function'
    && typeof storage.setItem === 'function';
  let persisted = createEmptySchema();

  function load(): AutocompleteSchema {
    if (!canStore || !storage) {
      persisted = createEmptySchema();
      return persisted;
    }

    try {
      const raw = storage.getItem(storageKey);
      persisted = normalizeHistory(raw ? JSON.parse(raw) as string | number | boolean | null | undefined | object : null);
    } catch {
      persisted = createEmptySchema();
    }

    return persisted;
  }

  function save(): void {
    if (!canStore || !storage) {
      return;
    }

    try {
      storage.setItem(storageKey, JSON.stringify(persisted));
    } catch {
    }
  }

  function getSchema(): AutocompleteSchema {
    return persisted;
  }

  function rememberToken(kind: TokenKind, value: string | number | boolean | null | undefined, key = ''): void {
    const text = String(value || '').trim();
    if (!text) {
      return;
    }

    if (kind === 'block') {
      const blockType = text.replace(/^::/, '').split(/\s+/)[0];
      addUniqueItems(persisted.blockTypes, [blockType]);
      if (persisted.blockTypes.length > AUTOCOMPLETE_HISTORY_MAX_ITEMS) {
        persisted.blockTypes = persisted.blockTypes.slice(-AUTOCOMPLETE_HISTORY_MAX_ITEMS);
      }
      save();
      return;
    }

    if (kind === 'attribute-key') {
      const attributeKey = text.replace(/=$/, '').toLowerCase();
      addUniqueItems(persisted.attributeKeys, [attributeKey]);
      if (persisted.attributeKeys.length > AUTOCOMPLETE_HISTORY_MAX_ITEMS) {
        persisted.attributeKeys = persisted.attributeKeys.slice(-AUTOCOMPLETE_HISTORY_MAX_ITEMS);
      }
      save();
      return;
    }

    if (kind === 'attribute-value') {
      const attributeKey = String(key || '').toLowerCase();
      if (!attributeKey) {
        return;
      }

      if (!Array.isArray(persisted.attributeValuesByKey[attributeKey])) {
        persisted.attributeValuesByKey[attributeKey] = [];
      }

      addUniqueItems(persisted.attributeValuesByKey[attributeKey], [text]);
      if (persisted.attributeValuesByKey[attributeKey].length > AUTOCOMPLETE_HISTORY_MAX_ITEMS) {
        persisted.attributeValuesByKey[attributeKey] = persisted.attributeValuesByKey[attributeKey].slice(-AUTOCOMPLETE_HISTORY_MAX_ITEMS);
      }
      save();
    }
  }

  load();

  return {
    getSchema,
    load,
    rememberToken,
  };
}
