import { signal } from "nsh";

/** 任务页上同一时间只会有一个盖在列表上的菜单。 */
export type ActiveMenu =
  | { readonly kind: "import" }
  | { readonly kind: "sort" }
  | { readonly kind: "item"; readonly key: string };

export const activeMenu = signal<ActiveMenu | null>(null);

export const openMenu = (menu: ActiveMenu) => activeMenu.set(menu);

export const closeMenu = () => activeMenu.set(null);
