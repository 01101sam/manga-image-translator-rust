import { keymap, platform, Text, View, type JSX } from "nsh";
import { Button as BaseButton } from "nsh/base";
import { Icon, type IconName } from "./icons";

const desktop = platform.desktop;

/**
 * 盖在页面上的一张居中卡片, 点卡片外或按 Esc 收起。
 * 嵌入 iOS 的运行时没有 Menu、Popover、Sheet (client-ui/HOST_PROTOCOL.md), 所以任务页的菜单都自己画。
 */
export function MenuCard(props: { testID: string; backdropTestID: string; title: string; onClose: () => void; children: JSX.Element }) {
  keymap.bind("escape", props.onClose);
  return (
    <View testID={props.testID} class="absolute inset-0 items-center justify-center bg-foreground/30 p-6">
      <BaseButton testID={props.backdropTestID} accessibilityLabel="关闭" onPress={props.onClose} class="absolute inset-0" />
      <View class="w-72 gap-1 rounded-(--radius) border border-border bg-card p-2">
        <Text class="truncate px-3 pb-1 pt-2 text-sm font-medium text-muted-foreground">{props.title}</Text>
        {props.children}
      </View>
    </View>
  );
}

export function MenuEntry(props: {
  testID: string;
  label: string;
  icon: IconName;
  onPress: () => void;
  disabled?: boolean;
  /** 右侧的小字, 例如快捷键。 */
  hint?: string;
}) {
  return (
    <BaseButton
      testID={props.testID}
      accessibilityLabel={props.label}
      disabled={props.disabled}
      onPress={props.onPress}
      class={`flex-row items-center gap-3 rounded-(--radius) px-3 hover:bg-accent active:opacity-70 disabled:opacity-40 focus-visible:ring-2 focus-visible:ring-ring ${desktop ? "h-9" : "h-12"}`}
    >
      <Icon name={props.icon} size={desktop ? 16 : 20} />
      <Text class={`flex-1 text-foreground ${desktop ? "text-sm" : "text-base"}`}>{props.label}</Text>
      <If when={props.hint}>{(hint) => <Text class="text-xs text-muted-foreground">{hint()}</Text>}</If>
    </BaseButton>
  );
}