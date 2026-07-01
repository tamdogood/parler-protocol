/// <reference types="vite/client" />
import type { ParlerApi } from "@shared/types";

declare global {
  interface Window {
    parler: ParlerApi;
  }
}

export {};
