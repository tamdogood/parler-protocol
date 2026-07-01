import type { ParlerApi } from "@shared/types";

/** The preload-exposed bridge to the main process. */
export const parler: ParlerApi = window.parler;
