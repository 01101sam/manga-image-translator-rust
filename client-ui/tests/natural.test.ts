import { describe, expect, test } from "nsh/test";
import { createNaturalCompare, detectCollator, naturalCompare, naturalKey } from "../src/domain/natural";

const sorted = (names: string[], compare = naturalCompare) => [...names].sort(compare);

describe("natural order", () => {
  test("page1, page2, page3, page10 sort by number, not by character", () => {
    const names = ["page10", "page2", "page1", "page3", "page11", "page20"];
    expect(sorted(names)).toEqual(["page1", "page2", "page3", "page10", "page11", "page20"]);
    // 默认的字典序会把 page10 排在 page2 前面, 这正是要避免的。
    expect([...names].sort()).not.toEqual(sorted(names));
  });

  test("every number in the name is compared in turn", () => {
    expect(sorted(["vol10-ch1", "vol2-ch10", "vol2-ch9", "vol1-ch100"])).toEqual(["vol1-ch100", "vol2-ch9", "vol2-ch10", "vol10-ch1"]);
    expect(sorted(["a2b1", "a1b10", "a1b2"])).toEqual(["a1b2", "a1b10", "a2b1"]);
    expect(sorted(["1-2-10", "1-10-1", "1-2-9"])).toEqual(["1-2-9", "1-2-10", "1-10-1"]);
  });

  test("case does not matter", () => {
    expect(sorted(["Page10", "page2", "PAGE1"])).toEqual(["PAGE1", "page2", "Page10"]);
    expect(sorted(["b.png", "A.png", "c.png", "B2.png"])).toEqual(["A.png", "b.png", "B2.png", "c.png"]);
    expect(naturalCompare("Page1", "page1")).toBe(0);
  });

  test("leading zeros count as the same number, then fewer zeros come first", () => {
    expect(naturalCompare("page01", "page1")).toBeGreaterThan(0);
    expect(sorted(["page001", "page01", "page1", "page2"])).toEqual(["page1", "page01", "page001", "page2"]);
    expect(sorted(["page10", "page002", "page02", "page9"])).toEqual(["page02", "page002", "page9", "page10"]);
    // 数值不同时前导零不起作用
    expect(sorted(["p010", "p9", "p0011"])).toEqual(["p9", "p010", "p0011"]);
    expect(sorted(["a01b2", "a1b10", "a1b1"])).toEqual(["a1b1", "a01b2", "a1b10"]);
    expect(sorted(["0", "00", "1", "000"])).toEqual(["0", "00", "000", "1"]);
  });

  test("numbers longer than a double keep their exact order", () => {
    const a = "x99999999999999999999";
    const b = "x100000000000000000000";
    expect(sorted([b, a])).toEqual([a, b]);
    expect(naturalCompare("x12345678901234567890", "x12345678901234567891")).toBeLessThan(0);
  });

  test("a name continuing with a symbol comes before one continuing with a number: page.png before page1.png", () => {
    expect(sorted(["page2.png", "page10.png", "page.png", "page1.png"])).toEqual(["page.png", "page1.png", "page2.png", "page10.png"]);
    expect(sorted(["ab1", "abc", "ab-", "ab"])).toEqual(["ab", "ab-", "ab1", "abc"]);
    expect(sorted(["a 2", "a2", "a 10", "a"])).toEqual(["a", "a 2", "a 10", "a2"]);
  });

  test("names with the same prefix: the shorter one first", () => {
    expect(sorted(["page1.png", "page", "page1", "page1a", "pag"])).toEqual(["pag", "page", "page1", "page1.png", "page1a"]);
  });

  test("names without digits sort alphabetically, ignoring case", () => {
    expect(sorted(["cherry", "Banana", "apple", "banana2"])).toEqual(["apple", "Banana", "banana2", "cherry"]);
    expect(sorted(["", "a"])).toEqual(["", "a"]);
  });

  test("a digit run against a text run compares as text, so 1a comes before a1", () => {
    expect(sorted(["a1", "1a", "-1"])).toEqual(["-1", "1a", "a1"]);
  });

  test("full-width digits are numbers too", () => {
    expect(sorted(["ｐａｇｅ１０", "page2", "page1"])).toEqual(["page1", "page2", "ｐａｇｅ１０"]);
    expect(naturalKey("１２").map((t) => t.value)).toEqual(["12"]);
  });

  test("Chinese and Japanese names: numbers inside are still numeric", () => {
    expect(sorted(["第10话", "第2话", "第1话"])).toEqual(["第1话", "第2话", "第10话"]);
    expect(sorted(["ページ10", "ページ2", "ページ1"])).toEqual(["ページ1", "ページ2", "ページ10"]);
    expect(sorted(["第2巻第10話", "第2巻第9話", "第10巻第1話"])).toEqual(["第2巻第9話", "第2巻第10話", "第10巻第1話"]);
  });

  test("without a collator, hiragana and katakana of the same sound are equal and kana keep gojuon order", () => {
    expect(createNaturalCompare()("あ1", "ア1")).toBe(0);
    expect(createNaturalCompare()("ｱ1", "あ1")).toBe(0);
    expect(sorted(["う", "い", "あ", "え"], createNaturalCompare())).toEqual(["あ", "い", "う", "え"]);
    expect(sorted(["ウ2", "あ2", "い2", "ア10"], createNaturalCompare())).toEqual(["あ2", "ア10", "い2", "ウ2"]);
  });

  test("text runs go through the engine's collator when there is one, numbers never do", () => {
    const seen: string[] = [];
    const reversed = createNaturalCompare({
      compare: (a, b) => {
        seen.push(a + "|" + b);
        return a < b ? 1 : a > b ? -1 : 0;
      },
    });
    expect(sorted(["a2", "b1", "a10"], reversed)).toEqual(["b1", "a2", "a10"]);
    expect(seen.length).toBeGreaterThan(0);
    // 数字段只拿数字段互相比, 从不交给 collator
    expect(seen.every((pair) => !/^\d+\|\d+$/.test(pair))).toBe(true);
  });

  test("the comparator is a consistent order: shuffles of the same names sort the same", () => {
    const names = ["a1", "a01", "A1", "a2", "a10", "b", "B", "page1", "page10", "page2", "第1话", "第10话", "第2话", ""];
    const expected = sorted(names);
    for (let seed = 1; seed <= 20; seed++) {
      const shuffled = names
        .map((name, index) => ({ name, rank: ((index + 1) * seed * 7919) % 101 }))
        .sort((x, y) => x.rank - y.rank)
        .map((entry) => entry.name);
      const result = sorted(shuffled);
      // 只看比较结果相同的是否一致: 大小写不同的名字比较为 0, 彼此的顺序取决于输入, 所以按小写折叠后比。
      expect(result.map((n) => n.toLowerCase())).toEqual(expected.map((n) => n.toLowerCase()));
    }
  });

  test("detecting the engine's collator never throws", () => {
    const collator = detectCollator();
    if (collator) expect(typeof collator.compare("a", "b")).toBe("number");
    else expect(collator).toBeUndefined();
  });
});