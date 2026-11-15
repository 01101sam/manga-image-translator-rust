import { platform, Text, View, type JSX } from "nsh";
import { Progress } from "nsh/base";
import { Button } from "./button";
import { Icon, type IconName } from "./icons";

const desktop = platform.desktop;

export function Card(props: { children: JSX.Element; class?: string }) {
  return <View class={`gap-3 rounded-(--radius) border border-border bg-card p-4 ${props.class ?? ""}`}>{props.children}</View>;
}

export type BadgeTone = "neutral" | "primary" | "success" | "warning" | "destructive";

const badgeStyle: Record<BadgeTone, { box: string; label: string }> = {
  neutral: { box: "bg-muted", label: "text-muted-foreground" },
  primary: { box: "bg-primary/15", label: "text-primary" },
  success: { box: "bg-success/15", label: "text-success" },
  warning: { box: "bg-warning/15", label: "text-warning" },
  destructive: { box: "bg-destructive/15", label: "text-destructive" },
};

export function Badge(props: { label: string; tone?: BadgeTone; testID?: string }) {
  const tone = () => badgeStyle[props.tone ?? "neutral"];
  return (
    <View testID={props.testID} class={`rounded-full px-2.5 py-0.5 ${tone().box}`}>
      <Text class={`text-xs font-medium ${tone().label}`}>{props.label}</Text>
    </View>
  );
}

export function Separator() {
  return <View class="h-px bg-border" />;
}

/** 空状态: 图标 + 标题 + 说明 + 可选的主操作。 */
export function Empty(props: {
  icon: IconName;
  title: string;
  description?: string;
  actionLabel?: string;
  actionTestID?: string;
  onAction?: () => void;
}) {
  return (
    <View class="flex-1 items-center justify-center gap-3 p-8">
      <View class="size-14 items-center justify-center rounded-full bg-muted">
        <Icon name={props.icon} size={26} tone="muted-foreground" />
      </View>
      <Text class="text-center text-lg font-semibold text-foreground">{props.title}</Text>
      <If when={props.description}>
        <Text class="max-w-80 text-center text-sm text-muted-foreground">{props.description}</Text>
      </If>
      <If when={props.actionLabel}>
        <Button label={props.actionLabel ?? ""} testID={props.actionTestID} onPress={() => props.onAction?.()} icon="plus" />
      </If>
    </View>
  );
}

/** 不确定进度的细条, 用于加载中。 */
export function Spinner(props: { label?: string }) {
  return (
    <View class="items-center gap-2" accessibilityLabel={props.label ?? "加载中"}>
      <Progress indeterminate trackClass="h-1 w-24 overflow-hidden rounded-full bg-muted" indicatorClass="h-1 rounded-full bg-primary" />
      <If when={props.label}>
        <Text class="text-sm text-muted-foreground">{props.label}</Text>
      </If>
    </View>
  );
}

/** 有进度的细条 (0..1)。 */
export function Meter(props: { value: number; testID?: string }) {
  return (
    <Progress
      testID={props.testID}
      value={props.value * 100}
      trackClass={`overflow-hidden rounded-full bg-muted ${desktop ? "h-1.5" : "h-2"}`}
      indicatorClass="h-full rounded-full bg-primary"
    />
  );
}

/** 页面顶部的一条提示, 可带一个操作。 */
export function Banner(props: { text: string; tone?: "warning" | "destructive"; actionLabel?: string; onAction?: () => void; testID?: string }) {
  const warn = () => (props.tone ?? "warning") === "warning";
  return (
    <View
      testID={props.testID}
      class={`flex-row items-center gap-3 border-b px-4 py-2.5 ${warn() ? "border-warning/30 bg-warning/10" : "border-destructive/30 bg-destructive/10"}`}
    >
      <Icon name="alert" size={18} tone={warn() ? "warning" : "destructive"} />
      <Text class="flex-1 text-sm text-foreground">{props.text}</Text>
      <If when={props.actionLabel}>
        <Button label={props.actionLabel ?? ""} size="sm" variant="secondary" onPress={() => props.onAction?.()} />
      </If>
    </View>
  );
}
