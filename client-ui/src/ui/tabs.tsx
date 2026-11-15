import { platform, Text } from "nsh";
import { Tab, Tabs } from "nsh/base";
import { Icon, type IconName } from "./icons";

const desktop = platform.desktop;

export interface TabItem<T extends string> {
  readonly value: T;
  readonly label: string;
  readonly icon?: IconName;
}

/** 分段控件: 少量互斥选项 (原文/译文)。 */
export function Segmented<T extends string>(props: { items: readonly TabItem<T>[]; value: T; onChange: (value: T) => void; label: string; testID?: string }) {
  return (
    <Tabs
      testID={props.testID}
      accessibilityLabel={props.label}
      class="flex-row gap-1 self-start rounded-full bg-muted p-1"
      indicatorClass="top-1 bottom-1 rounded-full bg-background shadow-sm"
    >
      <Each of={props.items} key={(item) => item.value}>
        {(item) => (
          <Tab
            testID={`${props.testID ?? "segmented"}-${item().value}`}
            accessibilityLabel={item().label}
            selected={props.value === item().value}
            onPress={() => props.onChange(item().value)}
            class={`group items-center justify-center rounded-full px-4 ${desktop ? "h-7" : "h-9"}`}
          >
            <Text class="text-sm font-medium text-muted-foreground group-selected:text-foreground">{item().label}</Text>
          </Tab>
        )}
      </Each>
    </Tabs>
  );
}

/** 顶层导航: 触屏在底部, 桌面与 Web 在顶部, 都是同一组 Tab。 */
export function NavTabs<T extends string>(props: { items: readonly TabItem<T>[]; value: T; onChange: (value: T) => void }) {
  return (
    <Tabs
      accessibilityLabel="主导航"
      class={desktop ? "flex-row gap-1" : "flex-row border-t border-border bg-card pb-[env(safe-area-inset-bottom)]"}
    >
      <Each of={props.items} key={(item) => item.value}>
        {(item) => (
          <Tab
            testID={`tab-${item().value}`}
            accessibilityLabel={item().label}
            selected={props.value === item().value}
            onPress={() => props.onChange(item().value)}
            class={
              desktop
                ? "group h-8 flex-row items-center gap-2 rounded-md px-3 hover:bg-accent selected:bg-accent"
                : "group h-14 flex-1 items-center justify-center gap-0.5"
            }
          >
            <Icon name={item().icon ?? "list"} size={desktop ? 16 : 22} tone={props.value === item().value ? "primary" : "muted-foreground"} />
            <Text
              class={
                desktop
                  ? "text-sm font-medium text-muted-foreground group-selected:text-foreground"
                  : "text-xs font-medium text-muted-foreground group-selected:text-primary"
              }
            >
              {item().label}
            </Text>
          </Tab>
        )}
      </Each>
    </Tabs>
  );
}
