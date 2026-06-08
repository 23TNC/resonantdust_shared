/* tslint:disable */
/* eslint-disable */

/**
 * A loaded content runtime: the [`Bundle`] plus the operations the client
 * calls (card render-view, client-side recipe match/plan). Opaque to JS —
 * constructed once from the `.rd` sources, then queried.
 */
export class Content {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * `allBlueprints()` → the blueprint catalog as JSON.
     */
    allBlueprints(): string;
    /**
     * `allTextures()` → the texture registry as JSON.
     */
    allTextures(): string;
    /**
     * `aspectInfo(name)` → the aspect record as JSON (`null` if unknown).
     */
    aspectInfo(name: string): string;
    /**
     * `aspectNames()` → all aspect names, sorted (client builds its id map).
     */
    aspectNames(): string[];
    /**
     * `aspectValue(packed, name)` → the folded magnitude (or `undefined`).
     */
    aspectValue(packed: number, name: string): bigint | undefined;
    /**
     * `cardDef(packed)` → the render def as JSON (`null` if unknown).
     */
    cardDef(packed: number): string;
    cardDefId(name: string): number | undefined;
    /**
     * `cardView(cardJson)` → the view cell as JSON. `cardJson` = `{def_id, stock}`.
     */
    cardView(card_json: string): string;
    /**
     * `defIdForPacked(packed)` → the global card def id (or `undefined`).
     */
    defIdForPacked(packed: number): number | undefined;
    /**
     * `drawVisuals(packed, hostJson, hook)` → the `PrimList` JSON for a card's
     * `:visuals` hook. `hostJson` is a plain object of instance state
     * (`{"poison": 1, "faction": "chorus"}`) — numbers become Int/Float, strings
     * Sym. `hook` is "init" | "update".
     */
    drawVisuals(packed: number, host_json: string, hook: string): string;
    /**
     * `generateTile(q, r, seed)` → the packed Zone tile slot
     * `[def_id:u12 | stock0:u2 | stock1:u2]` for a world hex. `seed` is a JS
     * number (the world seed is small; values beyond 2^53 lose precision).
     */
    generateTile(q: number, r: number, seed: number): number;
    /**
     * `globals()` → the `<globals>` constants as a JSON `{ id: number }` map
     * (card_width, card_height, title_height, hex_*). The client reads card/cell
     * dimensions from here instead of hardcoding `RECT_CARD_*` / hex radius.
     */
    globals(): string;
    /**
     * `matchRecipe(placedJson, recipe)` → the `Plan` as JSON (`null` if no
     * such recipe). `placedJson` = `[[slotPath, {def_id, stock}], …]`.
     */
    matchRecipe(placed_json: string, recipe: string): string;
    /**
     * `new Content(sourcesJson)` — `sourcesJson` is `[[name, text], …]`.
     */
    constructor(sources_json: string);
    /**
     * `packedDef(name)` → the packed `[type:u4 | def_id:u12]` (or `undefined`).
     */
    packedDef(name: string): number | undefined;
    planRecipe(placed_json: string, recipe: string): string;
    recipeId(name: string): number | undefined;
    /**
     * `recipeMeta(name)` → the recipe's iterators + anchors as JSON (`null` if
     * unknown). Drives client discovery + binding construction.
     */
    recipeMeta(name: string): string;
    recipeName(id: number): string | undefined;
    /**
     * `recipeNames()` → the recipe candidate list (Bundle-id order).
     */
    recipeNames(): string[];
    /**
     * `tilePrims(packed, stock0, stock1, seed)` → the world tile's `PrimList`
     * JSON from its stored stock (the two zone stock slots). `seed` is the
     * tile's `(q,r)` hash, driving the `:visuals` scatter (ring angles, scale).
     */
    tilePrims(packed: number, stock0: number, stock1: number, seed: number): string;
    /**
     * Batched `tilePrims`: `reqs` is a flat `[packed, stock0, stock1, seed]`
     * quadruplet per tile. Returns a JSON array of per-tile PrimNode arrays in
     * request order — one boundary crossing + one serialize for a whole drain
     * batch instead of one call per tile.
     */
    tilePrimsBatch(reqs: Int32Array): string;
}

/**
 * Hot-loadable locale catalog — label / description / message strings the
 * client renders. Independent of the [`Content`] bundle so it can reload on its
 * own: the client fetches `content/locales/<domain>/<lang>.json` at runtime and
 * hands the text in, so editing a locale file and re-constructing picks up the
 * change without recompiling. Keys are flattened `domain.path`
 * (`cards.requisite.log.label`).
 */
export class Locales {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * `new Locales(sourcesJson)` — `sourcesJson` is `[[domain, json], …]`,
     * e.g. `[["cards", "{…}"], ["recipes", "{…}"]]`.
     */
    constructor(sources_json: string);
    /**
     * `string(key)` → the localized string, or `undefined`.
     */
    string(key: string): string | undefined;
}

export function cardFlagBit(name: string): number | undefined;

export function cardFlagBitIn(field: string, name: string): number | undefined;

export function cardFlagFieldShape(field: string, name: string): Uint8Array | undefined;

export function cardFlagFieldValueAny(flags: number, stock: number, name: string): number | undefined;

export function cardFlagFieldValueIn(field: string, host: number, name: string): number | undefined;

export function cardTypeId(name: string): number | undefined;

export function hasCardFlag(flags: number, flags_bk: number, name: string): boolean;

export function isHexType(type_id: number): boolean;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_content_free: (a: number, b: number) => void;
    readonly __wbg_locales_free: (a: number, b: number) => void;
    readonly cardFlagBit: (a: number, b: number) => number;
    readonly cardFlagBitIn: (a: number, b: number, c: number, d: number) => number;
    readonly cardFlagFieldShape: (a: number, b: number, c: number, d: number) => [number, number];
    readonly cardFlagFieldValueAny: (a: number, b: number, c: number, d: number) => number;
    readonly cardFlagFieldValueIn: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly cardTypeId: (a: number, b: number) => number;
    readonly content_allBlueprints: (a: number) => [number, number, number, number];
    readonly content_allTextures: (a: number) => [number, number, number, number];
    readonly content_aspectInfo: (a: number, b: number, c: number) => [number, number, number, number];
    readonly content_aspectNames: (a: number) => [number, number];
    readonly content_aspectValue: (a: number, b: number, c: number, d: number) => [number, bigint];
    readonly content_cardDef: (a: number, b: number) => [number, number, number, number];
    readonly content_cardDefId: (a: number, b: number, c: number) => number;
    readonly content_cardView: (a: number, b: number, c: number) => [number, number, number, number];
    readonly content_defIdForPacked: (a: number, b: number) => number;
    readonly content_drawVisuals: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number, number];
    readonly content_generateTile: (a: number, b: number, c: number, d: number) => number;
    readonly content_globals: (a: number) => [number, number, number, number];
    readonly content_matchRecipe: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly content_new: (a: number, b: number) => [number, number, number];
    readonly content_packedDef: (a: number, b: number, c: number) => number;
    readonly content_planRecipe: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly content_recipeId: (a: number, b: number, c: number) => number;
    readonly content_recipeMeta: (a: number, b: number, c: number) => [number, number, number, number];
    readonly content_recipeName: (a: number, b: number) => [number, number];
    readonly content_recipeNames: (a: number) => [number, number];
    readonly content_tilePrims: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly content_tilePrimsBatch: (a: number, b: number, c: number) => [number, number, number, number];
    readonly hasCardFlag: (a: number, b: number, c: number, d: number) => number;
    readonly isHexType: (a: number) => number;
    readonly locales_new: (a: number, b: number) => [number, number, number];
    readonly locales_string: (a: number, b: number, c: number) => [number, number];
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __externref_drop_slice: (a: number, b: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
