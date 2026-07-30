/// <reference types="vite/client" />

declare module "*.vue" {
  import type { DefineComponent } from "vue";
  const component: DefineComponent<{}, {}, any>;
  export default component;
}

// `resolveJsonModule` would otherwise make TypeScript infer a literal type for
// all ~24k entries of the bundled game list, which is a large and pointless
// cost on every `vue-tsc` run. Declaring the module up front short-circuits it.
declare module "@/assets/gamelist.json" {
  const games: import("@/types/types").Game[];
  export default games;
}
