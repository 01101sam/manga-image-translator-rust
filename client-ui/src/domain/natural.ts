/**
 * 文件名的自然序: 数字段按数值比 (page2 在 page10 前), 其余文字不分大小写。
 *
 * 嵌入 App 的 JS 引擎没有可用的 Intl.Collator (实测 `new Intl.Collator` 抛错, localeCompare 的 numeric 选项被忽略),
 * 所以切段和数字比较都自己做, 只有「文字段怎么排」可以换成宿主引擎的 Collator (浏览器里有, 中日文按语言习惯排)。
 */

export interface TextCollator {
  compare(a: string, b: string): number;
}

interface Token {
  readonly digits: boolean;
  readonly text: string;
  /** 数字段去掉前导零后的值, 比较时先比长度再比字符, 任意长度都不丢精度。 */
  readonly value: string;
}

export type NaturalKey = readonly Token[];

/** NFKC 先把全角数字和半角片假名归一, 再切成数字段和非数字段。 */
export function naturalKey(name: string): NaturalKey {
  const parts = name.normalize("NFKC").match(/\d+|\D+/g) ?? [];
  return parts.map((text) => {
    const digits = text.charCodeAt(0) >= 48 && text.charCodeAt(0) <= 57;
    return { digits, text, value: digits ? text.replace(/^0+/, "") : text };
  });
}

/** 没有 Collator 时的文字比较: 小写, 片假名折成平假名 (あ 与 ア 同序), 按码点比。 */
function foldText(text: string): string {
  return text.toLowerCase().replace(/[ァ-ヶ]/g, (c) => String.fromCharCode(c.charCodeAt(0) - 0x60));
}

const foldedCompare: TextCollator["compare"] = (a, b) => {
  const x = foldText(a);
  const y = foldText(b);
  return x < y ? -1 : x > y ? 1 : 0;
};

/** 文字段后面紧跟的数字 (没有就是键结束了)。文字段总是连着数字段或键的结尾, 因为文字段取到最长。 */
const digitAfter = (key: NaturalKey, index: number): string | undefined => {
  const next = key[index + 1];
  return next?.digits ? next.text[0] : undefined;
};

/**
 * 两个文字段的比较。一个是另一个的前缀时 (a.png 对 a2.png: 文字段 "a.png" 与 "a"), 短的那个后面紧跟数字或结束,
 * 所以拿这个数字和长的下一个字符比, 而不是把短的当成更小: 这样 a.png 在 a2.png 前, 与逐字符比较一致。
 */
function compareText(x: Token, y: Token, after: { x?: string; y?: string }, text: TextCollator["compare"]): number {
  const fx = foldText(x.text);
  const fy = foldText(y.text);
  if (fx === fy) return 0;
  if (fy.length > fx.length && fy.startsWith(fx)) return after.x === undefined ? -1 : text(after.x, fy[fx.length]!);
  if (fx.length > fy.length && fx.startsWith(fy)) return after.y === undefined ? 1 : -text(after.y, fx[fy.length]!);
  return text(x.text, y.text);
}

/**
 * 逐段比较。数值全部相同时, 前导零少的在前 (page1 在 page01 前), 保证 page1、page01、page001 的顺序固定;
 * 仅大小写不同的名字比较结果为 0, 排序时由调用方保持原有顺序。
 */
export function compareKeys(a: NaturalKey, b: NaturalKey, text: TextCollator["compare"] = foldedCompare): number {
  const shared = Math.min(a.length, b.length);
  for (let i = 0; i < shared; i++) {
    const x = a[i]!;
    const y = b[i]!;
    let order: number;
    if (x.digits && y.digits) {
      order = x.value.length !== y.value.length ? (x.value.length < y.value.length ? -1 : 1) : x.value < y.value ? -1 : x.value > y.value ? 1 : 0;
    } else if (!x.digits && !y.digits) {
      order = compareText(x, y, { x: digitAfter(a, i), y: digitAfter(b, i) }, text);
    } else {
      order = text(x.text, y.text);
    }
    if (order !== 0) return order;
  }
  if (a.length !== b.length) return a.length < b.length ? -1 : 1;
  for (let i = 0; i < shared; i++) {
    const x = a[i]!;
    const y = b[i]!;
    if (x.digits && x.text.length !== y.text.length) return x.text.length < y.text.length ? -1 : 1;
  }
  return 0;
}

/** 宿主引擎有 Collator 就用它排文字段 (base: 忽略大小写和重音), 没有或不能用就返回 undefined。 */
export function detectCollator(): TextCollator | undefined {
  try {
    const Collator = (globalThis as unknown as { Intl?: { Collator?: new (locale?: string, options?: object) => TextCollator } }).Intl?.Collator;
    if (typeof Collator !== "function") return undefined;
    const collator = new Collator(undefined, { sensitivity: "base" });
    return typeof collator.compare("a", "b") === "number" ? collator : undefined;
  } catch {
    return undefined;
  }
}

/** 文字段交给 collator, 没有就用折叠后的码点序。 */
export function createKeyCompare(collator?: TextCollator): (a: NaturalKey, b: NaturalKey) => number {
  const text = collator ? (a: string, b: string) => collator.compare(a, b) : foldedCompare;
  return (a, b) => compareKeys(a, b, text);
}

export function createNaturalCompare(collator?: TextCollator): (a: string, b: string) => number {
  const compare = createKeyCompare(collator);
  return (a, b) => compare(naturalKey(a), naturalKey(b));
}

const detected = detectCollator();

export const naturalCompare = createNaturalCompare(detected);

/** 排序时先给每个名字算好键, 再用这个比较器。 */
export const compareNaturalKeys = createKeyCompare(detected);