import { keymap, platform, Text, View } from "nsh";
import { Button as BaseButton } from "nsh/base";
import { IMPORT_LABELS, type ImportSource } from "../domain/import";
import { closeImportMenu, runImport } from "../store/importer";
import { importSources } from "../store/session";
import { Icon, type IconName } from "../ui";

const desktop = platform.desktop;

const icons: Record<ImportSource, IconName> = { photos: "image", files: "file", folder: "folder", pdf: "doc", paste: "clipboard" };

function MenuItem(props: { source: ImportSource }) {
  return (
    <BaseButton
      testID={`import-${props.source}`}
      accessibilityLabel={IMPORT_LABELS[props.source]}
      onPress={() => runImport(props.source)}
      class={`flex-row items-center gap-3 rounded-(--radius) px-3 hover:bg-accent active:opacity-70 focus-visible:ring-2 focus-visible:ring-ring ${desktop ? "h-9" : "h-12"}`}
    >
      <Icon name={icons[props.source]} size={desktop ? 16 : 20} />
      <Text class={`flex-1 text-foreground ${desktop ? "text-sm" : "text-base"}`}>{IMPORT_LABELS[props.source]}</Text>
      <If when={props.source === "paste"}>
        <Text class="text-xs text-muted-foreground">{keymap.label("mod+v")}</Text>
      </If>
    </BaseButton>
  );
}

/**
 * 导入菜单: 盖在任务页上的一张居中卡片, 点卡片外或按 Esc 收起。
 * 嵌入 iOS 的运行时没有 Menu、Popover、Sheet (HOST_PROTOCOL.md), 所以自己画; 项目由宿主声明, 网页不会出现相册。
 */
export function ImportMenu() {
  keymap.bind("escape", closeImportMenu);
  return (
    <View testID="import-menu" class="absolute inset-0 items-center justify-center bg-foreground/30 p-6">
      <BaseButton testID="import-backdrop" accessibilityLabel="关闭" onPress={closeImportMenu} class="absolute inset-0" />
      <View class="w-72 gap-1 rounded-(--radius) border border-border bg-card p-2">
        <Text class="px-3 pb-1 pt-2 text-sm font-medium text-muted-foreground">导入</Text>
        <Each of={importSources()} key={(source) => source}>
          {(source) => <MenuItem source={source()} />}
        </Each>
      </View>
    </View>
  );
}
