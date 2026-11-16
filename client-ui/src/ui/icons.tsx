import { Canvas, Path2D, token } from "nsh";

/** 24 × 24 网格上的线条图标, 全部自绘, 不依赖外部图标库。 */
export type IconName =
  | "plus"
  | "x"
  | "check"
  | "chevron-left"
  | "chevron-right"
  | "image"
  | "file"
  | "folder"
  | "clipboard"
  | "list"
  | "sliders"
  | "doc"
  | "play"
  | "stop"
  | "refresh"
  | "eye"
  | "eye-off"
  | "trash"
  | "download"
  | "alert"
  | "clock"
  | "swap"
  | "external"
  | "sort"
  | "more"
  | "to-top"
  | "to-bottom";

type Shape =
  | { readonly path: string }
  | { readonly circle: readonly [number, number, number] }
  | { readonly rect: readonly [number, number, number, number, number] };

const shapes: Record<IconName, readonly Shape[]> = {
  plus: [{ path: "M12 5v14M5 12h14" }],
  x: [{ path: "M18 6 6 18M6 6l12 12" }],
  check: [{ path: "M5 12.5l4.5 4.5L19 7.5" }],
  "chevron-left": [{ path: "M15 5l-7 7 7 7" }],
  "chevron-right": [{ path: "M9 5l7 7-7 7" }],
  image: [{ rect: [3, 4, 18, 16, 3] }, { circle: [9, 10, 1.6] }, { path: "M3.5 17l5-5 4 4 3-3 5 4.5" }],
  file: [{ path: "M7 3h7l5 5v13H7z" }, { path: "M14 3v5h5" }],
  clipboard: [{ rect: [5, 5, 14, 16, 2] }, { path: "M9 5V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v1M9 11h6M9 15h4" }],
  folder: [{ path: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" }],
  list: [{ path: "M4 7h16M4 12h16M4 17h10" }],
  sliders: [{ path: "M4 7h9M17 7h3M4 17h3M11 17h9" }, { circle: [15, 7, 2] }, { circle: [9, 17, 2] }],
  doc: [{ path: "M7 3h7l5 5v13H7z" }, { path: "M14 3v5h5M10 13h6M10 17h6" }],
  play: [{ path: "M8 5l11 7-11 7z" }],
  stop: [{ rect: [6, 6, 12, 12, 2] }],
  refresh: [{ path: "M20 11a8 8 0 1 0-2.3 5.7" }, { path: "M20 4.5V11h-6.5" }],
  eye: [{ path: "M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z" }, { circle: [12, 12, 3] }],
  "eye-off": [{ path: "M2.5 12S6 5.5 12 5.5c2 0 3.8.7 5.3 1.7M21.5 12S18 18.5 12 18.5c-2 0-3.8-.7-5.3-1.7M4 4l16 16" }],
  trash: [{ path: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 11v6M14 11v6" }],
  download: [{ path: "M12 4v11M7 11l5 5 5-5M5 20h14" }],
  alert: [{ circle: [12, 12, 9] }, { path: "M12 7.5v5.5M12 16.5v.5" }],
  clock: [{ circle: [12, 12, 9] }, { path: "M12 7v5l3 2" }],
  swap: [{ path: "M4 8h14M14 4l4 4-4 4M20 16H6M10 12l-4 4 4 4" }],
  external: [{ path: "M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" }],
  sort: [{ path: "M8 5v14M4 15l4 4 4-4M16 19V5M12 9l4-4 4 4" }],
  more: [{ circle: [5, 12, 1.2] }, { circle: [12, 12, 1.2] }, { circle: [19, 12, 1.2] }],
  "to-top": [{ path: "M5 4h14M12 20V9M7 13l5-5 5 5" }],
  "to-bottom": [{ path: "M5 20h14M12 4v11M7 11l5 5 5-5" }],
};

export type IconTone = "foreground" | "muted-foreground" | "primary" | "primary-foreground" | "destructive" | "success" | "warning";

export function Icon(props: { name: IconName; size?: number; tone?: IconTone }) {
  const size = props.size ?? 20;
  return (
    <Canvas
      style={{ width: size, height: size, pointerEvents: "none" }}
      draw={(cx, { width }) => {
        cx.scale(width / 24, width / 24);
        cx.strokeStyle = token(props.tone ?? "foreground");
        cx.lineWidth = 2;
        cx.lineCap = "round";
        cx.lineJoin = "round";
        for (const shape of shapes[props.name]) {
          if ("path" in shape) cx.stroke(new Path2D(shape.path));
          else if ("circle" in shape) {
            cx.beginPath();
            cx.arc(shape.circle[0], shape.circle[1], shape.circle[2], 0, Math.PI * 2);
            cx.stroke();
          } else {
            cx.beginPath();
            cx.roundRect(shape.rect[0], shape.rect[1], shape.rect[2], shape.rect[3], shape.rect[4]);
            cx.stroke();
          }
        }
      }}
    />
  );
}