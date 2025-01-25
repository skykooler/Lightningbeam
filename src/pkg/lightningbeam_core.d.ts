/* tslint:disable */
/* eslint-disable */
export function main_js(): void;
export class CoreInterface {
  free(): void;
  constructor();
  init(): void;
  play(timestamp: number): void;
  stop(): void;
  resume_audio(): void;
  add_sine_track(frequency: number): void;
  get_timestamp(): number;
  get_tracks(): JsTrack[];
}
export class JsTrack {
  private constructor();
  free(): void;
  toString(): string;
  readonly name: string;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly __wbg_jstrack_free: (a: number, b: number) => void;
  readonly jstrack_name: (a: number, b: number) => void;
  readonly jstrack_toString: (a: number, b: number) => void;
  readonly __wbg_coreinterface_free: (a: number, b: number) => void;
  readonly coreinterface_new: () => number;
  readonly coreinterface_init: (a: number) => void;
  readonly coreinterface_play: (a: number, b: number) => void;
  readonly coreinterface_stop: (a: number) => void;
  readonly coreinterface_resume_audio: (a: number, b: number) => void;
  readonly coreinterface_add_sine_track: (a: number, b: number, c: number) => void;
  readonly coreinterface_get_timestamp: (a: number) => number;
  readonly coreinterface_get_tracks: (a: number, b: number) => void;
  readonly main_js: () => void;
  readonly __wbindgen_exn_store: (a: number) => void;
  readonly __wbindgen_export_1: WebAssembly.Table;
  readonly __wbindgen_malloc: (a: number, b: number) => number;
  readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
  readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
  readonly __wbindgen_free: (a: number, b: number, c: number) => void;
  readonly _dyn_core__ops__function__FnMut_____Output___R_as_wasm_bindgen__closure__WasmClosure___describe__invoke__h03a328ab39659ec3: (a: number, b: number) => void;
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
