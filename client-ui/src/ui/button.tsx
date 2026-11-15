import { platform, Text } from "nsh";
import { Button as BaseButton } from "nsh/base";
import { Icon, type IconName, type IconTone } from "./icons";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "destructive";
export type ButtonSize = "sm" | "md" | "lg";

const desktop = platform.desktop;

const base =
  "flex-row items-center justify-center gap-2 rounded-(--radius) active:opacity-70 disabled:opacity-40 focus-visible:ring-2 focus-visible:ring-ring";

const variants: Record<ButtonVariant, string> = {
  primary: "bg-primary hover:opacity-90",
  secondary: "bg-secondary hover:bg-accent",
  ghost: "hover:bg-accent",
  destructive: "bg-destructive hover:opacity-90",
};

const labelClass: Record<ButtonVariant, string> = {
  primary: "text-primary-foreground",
  secondary: "text-secondary-foreground",
  ghost: "text-foreground",
  destructive: "text-destructive-foreground",
};

const iconTone: Record<ButtonVariant, IconTone> = {
  primary: "primary-foreground",
  secondary: "foreground",
  ghost: "foreground",
  destructive: "primary-foreground",
};

// 触屏控件不小于 44pt, 桌面与 Web 用紧凑的 32pt。
const sizes: Record<ButtonSize, string> = desktop
  ? { sm: "h-7 px-2.5", md: "h-8 px-3", lg: "h-10 px-4" }
  : { sm: "h-9 px-3", md: "h-11 px-4", lg: "h-12 px-5" };

const labelSizes: Record<ButtonSize, string> = desktop
  ? { sm: "text-xs font-medium", md: "text-sm font-medium", lg: "text-sm font-medium" }
  : { sm: "text-sm font-medium", md: "text-base font-medium", lg: "text-base font-medium" };

export function Button(props: {
  label: string;
  onPress: () => void;
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: IconName;
  disabled?: boolean;
  loading?: boolean;
  testID?: string;
  class?: string;
}) {
  const variant = () => props.variant ?? "primary";
  const size = () => props.size ?? "md";
  return (
    <BaseButton
      testID={props.testID}
      accessibilityLabel={props.label}
      disabled={props.disabled || props.loading}
      onPress={props.onPress}
      class={`${base} ${variants[variant()]} ${sizes[size()]} ${props.class ?? ""}`}
    >
      <If when={props.icon}>{(name) => <Icon name={name()} size={desktop ? 16 : 18} tone={iconTone[variant()]} />}</If>
      <Text class={`${labelClass[variant()]} ${labelSizes[size()]}`}>{props.loading ? props.label + "…" : props.label}</Text>
    </BaseButton>
  );
}

/** 只有图标的按钮, 必须给出 label 供读屏与测试使用。 */
export function IconButton(props: {
  label: string;
  icon: IconName;
  onPress: () => void;
  tone?: IconTone;
  disabled?: boolean;
  testID?: string;
}) {
  return (
    <BaseButton
      testID={props.testID}
      accessibilityLabel={props.label}
      disabled={props.disabled}
      onPress={props.onPress}
      hitSlop={8}
      class={`items-center justify-center rounded-full hover:bg-accent active:opacity-60 disabled:opacity-40 focus-visible:ring-2 focus-visible:ring-ring ${desktop ? "size-8" : "size-11"}`}
    >
      <Icon name={props.icon} size={desktop ? 18 : 22} tone={props.tone ?? "foreground"} />
    </BaseButton>
  );
}
