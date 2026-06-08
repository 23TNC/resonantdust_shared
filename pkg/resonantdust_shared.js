/* @ts-self-types="./resonantdust_shared.d.ts" */

/**
 * A loaded content runtime: the [`Bundle`] plus the operations the client
 * calls (card render-view, client-side recipe match/plan). Opaque to JS —
 * constructed once from the `.rd` sources, then queried.
 */
export class Content {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        ContentFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_content_free(ptr, 0);
    }
    /**
     * `allBlueprints()` → the blueprint catalog as JSON.
     * @returns {string}
     */
    allBlueprints() {
        let deferred2_0;
        let deferred2_1;
        try {
            const ret = wasm.content_allBlueprints(this.__wbg_ptr);
            var ptr1 = ret[0];
            var len1 = ret[1];
            if (ret[3]) {
                ptr1 = 0; len1 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred2_0 = ptr1;
            deferred2_1 = len1;
            return getStringFromWasm0(ptr1, len1);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * `allTextures()` → the texture registry as JSON.
     * @returns {string}
     */
    allTextures() {
        let deferred2_0;
        let deferred2_1;
        try {
            const ret = wasm.content_allTextures(this.__wbg_ptr);
            var ptr1 = ret[0];
            var len1 = ret[1];
            if (ret[3]) {
                ptr1 = 0; len1 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred2_0 = ptr1;
            deferred2_1 = len1;
            return getStringFromWasm0(ptr1, len1);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * `aspectInfo(name)` → the aspect record as JSON (`null` if unknown).
     * @param {string} name
     * @returns {string}
     */
    aspectInfo(name) {
        let deferred3_0;
        let deferred3_1;
        try {
            const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.content_aspectInfo(this.__wbg_ptr, ptr0, len0);
            var ptr2 = ret[0];
            var len2 = ret[1];
            if (ret[3]) {
                ptr2 = 0; len2 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred3_0 = ptr2;
            deferred3_1 = len2;
            return getStringFromWasm0(ptr2, len2);
        } finally {
            wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
        }
    }
    /**
     * `aspectNames()` → all aspect names, sorted (client builds its id map).
     * @returns {string[]}
     */
    aspectNames() {
        const ret = wasm.content_aspectNames(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * `aspectValue(packed, name)` → the folded magnitude (or `undefined`).
     * @param {number} packed
     * @param {string} name
     * @returns {bigint | undefined}
     */
    aspectValue(packed, name) {
        const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.content_aspectValue(this.__wbg_ptr, packed, ptr0, len0);
        return ret[0] === 0 ? undefined : ret[1];
    }
    /**
     * `cardDef(packed)` → the render def as JSON (`null` if unknown).
     * @param {number} packed
     * @returns {string}
     */
    cardDef(packed) {
        let deferred2_0;
        let deferred2_1;
        try {
            const ret = wasm.content_cardDef(this.__wbg_ptr, packed);
            var ptr1 = ret[0];
            var len1 = ret[1];
            if (ret[3]) {
                ptr1 = 0; len1 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred2_0 = ptr1;
            deferred2_1 = len1;
            return getStringFromWasm0(ptr1, len1);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * @param {string} name
     * @returns {number | undefined}
     */
    cardDefId(name) {
        const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.content_cardDefId(this.__wbg_ptr, ptr0, len0);
        return ret === 0xFFFFFF ? undefined : ret;
    }
    /**
     * `cardView(cardJson)` → the view cell as JSON. `cardJson` = `{def_id, stock}`.
     * @param {string} card_json
     * @returns {string}
     */
    cardView(card_json) {
        let deferred3_0;
        let deferred3_1;
        try {
            const ptr0 = passStringToWasm0(card_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.content_cardView(this.__wbg_ptr, ptr0, len0);
            var ptr2 = ret[0];
            var len2 = ret[1];
            if (ret[3]) {
                ptr2 = 0; len2 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred3_0 = ptr2;
            deferred3_1 = len2;
            return getStringFromWasm0(ptr2, len2);
        } finally {
            wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
        }
    }
    /**
     * `defIdForPacked(packed)` → the global card def id (or `undefined`).
     * @param {number} packed
     * @returns {number | undefined}
     */
    defIdForPacked(packed) {
        const ret = wasm.content_defIdForPacked(this.__wbg_ptr, packed);
        return ret === 0xFFFFFF ? undefined : ret;
    }
    /**
     * `drawVisuals(packed, hostJson, hook)` → the `PrimList` JSON for a card's
     * `:visuals` hook. `hostJson` is a plain object of instance state
     * (`{"poison": 1, "faction": "chorus"}`) — numbers become Int/Float, strings
     * Sym. `hook` is "init" | "update".
     * @param {number} packed
     * @param {string} host_json
     * @param {string} hook
     * @returns {string}
     */
    drawVisuals(packed, host_json, hook) {
        let deferred4_0;
        let deferred4_1;
        try {
            const ptr0 = passStringToWasm0(host_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(hook, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            const ret = wasm.content_drawVisuals(this.__wbg_ptr, packed, ptr0, len0, ptr1, len1);
            var ptr3 = ret[0];
            var len3 = ret[1];
            if (ret[3]) {
                ptr3 = 0; len3 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred4_0 = ptr3;
            deferred4_1 = len3;
            return getStringFromWasm0(ptr3, len3);
        } finally {
            wasm.__wbindgen_free(deferred4_0, deferred4_1, 1);
        }
    }
    /**
     * `generateTile(q, r, seed)` → the packed Zone tile slot
     * `[def_id:u12 | stock0:u2 | stock1:u2]` for a world hex. `seed` is a JS
     * number (the world seed is small; values beyond 2^53 lose precision).
     * @param {number} q
     * @param {number} r
     * @param {number} seed
     * @returns {number}
     */
    generateTile(q, r, seed) {
        const ret = wasm.content_generateTile(this.__wbg_ptr, q, r, seed);
        return ret;
    }
    /**
     * `globals()` → the `<globals>` constants as a JSON `{ id: number }` map
     * (card_width, card_height, title_height, hex_*). The client reads card/cell
     * dimensions from here instead of hardcoding `RECT_CARD_*` / hex radius.
     * @returns {string}
     */
    globals() {
        let deferred2_0;
        let deferred2_1;
        try {
            const ret = wasm.content_globals(this.__wbg_ptr);
            var ptr1 = ret[0];
            var len1 = ret[1];
            if (ret[3]) {
                ptr1 = 0; len1 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred2_0 = ptr1;
            deferred2_1 = len1;
            return getStringFromWasm0(ptr1, len1);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * `matchRecipe(placedJson, recipe)` → the `Plan` as JSON (`null` if no
     * such recipe). `placedJson` = `[[slotPath, {def_id, stock}], …]`.
     * @param {string} placed_json
     * @param {string} recipe
     * @returns {string}
     */
    matchRecipe(placed_json, recipe) {
        let deferred4_0;
        let deferred4_1;
        try {
            const ptr0 = passStringToWasm0(placed_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(recipe, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            const ret = wasm.content_matchRecipe(this.__wbg_ptr, ptr0, len0, ptr1, len1);
            var ptr3 = ret[0];
            var len3 = ret[1];
            if (ret[3]) {
                ptr3 = 0; len3 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred4_0 = ptr3;
            deferred4_1 = len3;
            return getStringFromWasm0(ptr3, len3);
        } finally {
            wasm.__wbindgen_free(deferred4_0, deferred4_1, 1);
        }
    }
    /**
     * `new Content(sourcesJson)` — `sourcesJson` is `[[name, text], …]`.
     * @param {string} sources_json
     */
    constructor(sources_json) {
        const ptr0 = passStringToWasm0(sources_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.content_new(ptr0, len0);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        this.__wbg_ptr = ret[0];
        ContentFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * `packedDef(name)` → the packed `[type:u4 | def_id:u12]` (or `undefined`).
     * @param {string} name
     * @returns {number | undefined}
     */
    packedDef(name) {
        const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.content_packedDef(this.__wbg_ptr, ptr0, len0);
        return ret === 0xFFFFFF ? undefined : ret;
    }
    /**
     * @param {string} placed_json
     * @param {string} recipe
     * @returns {string}
     */
    planRecipe(placed_json, recipe) {
        let deferred4_0;
        let deferred4_1;
        try {
            const ptr0 = passStringToWasm0(placed_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(recipe, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            const ret = wasm.content_planRecipe(this.__wbg_ptr, ptr0, len0, ptr1, len1);
            var ptr3 = ret[0];
            var len3 = ret[1];
            if (ret[3]) {
                ptr3 = 0; len3 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred4_0 = ptr3;
            deferred4_1 = len3;
            return getStringFromWasm0(ptr3, len3);
        } finally {
            wasm.__wbindgen_free(deferred4_0, deferred4_1, 1);
        }
    }
    /**
     * @param {string} name
     * @returns {number | undefined}
     */
    recipeId(name) {
        const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.content_recipeId(this.__wbg_ptr, ptr0, len0);
        return ret === 0xFFFFFF ? undefined : ret;
    }
    /**
     * `recipeMeta(name)` → the recipe's iterators + anchors as JSON (`null` if
     * unknown). Drives client discovery + binding construction.
     * @param {string} name
     * @returns {string}
     */
    recipeMeta(name) {
        let deferred3_0;
        let deferred3_1;
        try {
            const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.content_recipeMeta(this.__wbg_ptr, ptr0, len0);
            var ptr2 = ret[0];
            var len2 = ret[1];
            if (ret[3]) {
                ptr2 = 0; len2 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred3_0 = ptr2;
            deferred3_1 = len2;
            return getStringFromWasm0(ptr2, len2);
        } finally {
            wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
        }
    }
    /**
     * @param {number} id
     * @returns {string | undefined}
     */
    recipeName(id) {
        const ret = wasm.content_recipeName(this.__wbg_ptr, id);
        let v1;
        if (ret[0] !== 0) {
            v1 = getStringFromWasm0(ret[0], ret[1]).slice();
            wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        }
        return v1;
    }
    /**
     * `recipeNames()` → the recipe candidate list (Bundle-id order).
     * @returns {string[]}
     */
    recipeNames() {
        const ret = wasm.content_recipeNames(this.__wbg_ptr);
        var v1 = getArrayJsValueFromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * `tilePrims(packed, stock0, stock1, seed)` → the world tile's `PrimList`
     * JSON from its stored stock (the two zone stock slots). `seed` is the
     * tile's `(q,r)` hash, driving the `:visuals` scatter (ring angles, scale).
     * @param {number} packed
     * @param {number} stock0
     * @param {number} stock1
     * @param {number} seed
     * @returns {string}
     */
    tilePrims(packed, stock0, stock1, seed) {
        let deferred2_0;
        let deferred2_1;
        try {
            const ret = wasm.content_tilePrims(this.__wbg_ptr, packed, stock0, stock1, seed);
            var ptr1 = ret[0];
            var len1 = ret[1];
            if (ret[3]) {
                ptr1 = 0; len1 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred2_0 = ptr1;
            deferred2_1 = len1;
            return getStringFromWasm0(ptr1, len1);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * Batched `tilePrims`: `reqs` is a flat `[packed, stock0, stock1, seed]`
     * quadruplet per tile. Returns a JSON array of per-tile PrimNode arrays in
     * request order — one boundary crossing + one serialize for a whole drain
     * batch instead of one call per tile.
     * @param {Int32Array} reqs
     * @returns {string}
     */
    tilePrimsBatch(reqs) {
        let deferred3_0;
        let deferred3_1;
        try {
            const ptr0 = passArray32ToWasm0(reqs, wasm.__wbindgen_malloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.content_tilePrimsBatch(this.__wbg_ptr, ptr0, len0);
            var ptr2 = ret[0];
            var len2 = ret[1];
            if (ret[3]) {
                ptr2 = 0; len2 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred3_0 = ptr2;
            deferred3_1 = len2;
            return getStringFromWasm0(ptr2, len2);
        } finally {
            wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
        }
    }
}
if (Symbol.dispose) Content.prototype[Symbol.dispose] = Content.prototype.free;

/**
 * Hot-loadable locale catalog — label / description / message strings the
 * client renders. Independent of the [`Content`] bundle so it can reload on its
 * own: the client fetches `content/locales/<domain>/<lang>.json` at runtime and
 * hands the text in, so editing a locale file and re-constructing picks up the
 * change without recompiling. Keys are flattened `domain.path`
 * (`cards.requisite.log.label`).
 */
export class Locales {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        LocalesFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_locales_free(ptr, 0);
    }
    /**
     * `new Locales(sourcesJson)` — `sourcesJson` is `[[domain, json], …]`,
     * e.g. `[["cards", "{…}"], ["recipes", "{…}"]]`.
     * @param {string} sources_json
     */
    constructor(sources_json) {
        const ptr0 = passStringToWasm0(sources_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.locales_new(ptr0, len0);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        this.__wbg_ptr = ret[0];
        LocalesFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * `string(key)` → the localized string, or `undefined`.
     * @param {string} key
     * @returns {string | undefined}
     */
    string(key) {
        const ptr0 = passStringToWasm0(key, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.locales_string(this.__wbg_ptr, ptr0, len0);
        let v2;
        if (ret[0] !== 0) {
            v2 = getStringFromWasm0(ret[0], ret[1]).slice();
            wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        }
        return v2;
    }
}
if (Symbol.dispose) Locales.prototype[Symbol.dispose] = Locales.prototype.free;

/**
 * @param {string} name
 * @returns {number | undefined}
 */
export function cardFlagBit(name) {
    const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.cardFlagBit(ptr0, len0);
    return ret === 0xFFFFFF ? undefined : ret;
}

/**
 * @param {string} field
 * @param {string} name
 * @returns {number | undefined}
 */
export function cardFlagBitIn(field, name) {
    const ptr0 = passStringToWasm0(field, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.cardFlagBitIn(ptr0, len0, ptr1, len1);
    return ret === 0xFFFFFF ? undefined : ret;
}

/**
 * @param {string} field
 * @param {string} name
 * @returns {Uint8Array | undefined}
 */
export function cardFlagFieldShape(field, name) {
    const ptr0 = passStringToWasm0(field, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.cardFlagFieldShape(ptr0, len0, ptr1, len1);
    let v3;
    if (ret[0] !== 0) {
        v3 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
    }
    return v3;
}

/**
 * @param {number} flags
 * @param {number} stock
 * @param {string} name
 * @returns {number | undefined}
 */
export function cardFlagFieldValueAny(flags, stock, name) {
    const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.cardFlagFieldValueAny(flags, stock, ptr0, len0);
    return ret === Number.MAX_SAFE_INTEGER ? undefined : ret;
}

/**
 * @param {string} field
 * @param {number} host
 * @param {string} name
 * @returns {number | undefined}
 */
export function cardFlagFieldValueIn(field, host, name) {
    const ptr0 = passStringToWasm0(field, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len1 = WASM_VECTOR_LEN;
    const ret = wasm.cardFlagFieldValueIn(ptr0, len0, host, ptr1, len1);
    return ret === Number.MAX_SAFE_INTEGER ? undefined : ret;
}

/**
 * @param {string} name
 * @returns {number | undefined}
 */
export function cardTypeId(name) {
    const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.cardTypeId(ptr0, len0);
    return ret === 0xFFFFFF ? undefined : ret;
}

/**
 * @param {number} flags
 * @param {number} flags_bk
 * @param {string} name
 * @returns {boolean}
 */
export function hasCardFlag(flags, flags_bk, name) {
    const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.hasCardFlag(flags, flags_bk, ptr0, len0);
    return ret !== 0;
}

/**
 * @param {number} type_id
 * @returns {boolean}
 */
export function isHexType(type_id) {
    const ret = wasm.isHexType(type_id);
    return ret !== 0;
}
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg___wbindgen_throw_1506f2235d1bdba0: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbindgen_cast_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./resonantdust_shared_bg.js": import0,
    };
}

const ContentFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_content_free(ptr, 1));
const LocalesFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_locales_free(ptr, 1));

function getArrayJsValueFromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    const mem = getDataViewMemory0();
    const result = [];
    for (let i = ptr; i < ptr + 4 * len; i += 4) {
        result.push(wasm.__wbindgen_externrefs.get(mem.getUint32(i, true)));
    }
    wasm.__externref_drop_slice(ptr, len);
    return result;
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint32ArrayMemory0 = null;
function getUint32ArrayMemory0() {
    if (cachedUint32ArrayMemory0 === null || cachedUint32ArrayMemory0.byteLength === 0) {
        cachedUint32ArrayMemory0 = new Uint32Array(wasm.memory.buffer);
    }
    return cachedUint32ArrayMemory0;
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function passArray32ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 4, 4) >>> 0;
    getUint32ArrayMemory0().set(arg, ptr / 4);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedUint32ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('resonantdust_shared_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
