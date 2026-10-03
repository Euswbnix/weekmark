import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  ASSESSMENT_WORDS,
  INCLUDABLE_REASON,
  looksLikeAssessment,
  STUDY_WORDS,
} from "./mock/explain";

// The facade's rules, in pagelamp-core: found from this test file, wherever the tests run from.
const testFile = expect.getState().testPath ?? "";
const core = (file: string) =>
  readFileSync(join(dirname(testFile), "../../../../crates/pagelamp-core/src", file), "utf8");
const builders = core("ai_gate/builders.rs");
const aiGate = core("ai_gate.rs");
const snake = (name: string) => name.replace(/(?<!^)([A-Z])/g, "_$1").toLowerCase();
const words = (constant: string) => {
  const list = builders.match(new RegExp(`const ${constant}: \\[&str; \\d+\\] = \\[([^\\]]*)\\]`));
  if (!list?.[1]) throw new Error(`${constant} not found in builders.rs`);
  return [...list[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
};

describe("the mock's include, against the facade", () => {
  it("marks includable exactly the reason week_context_including brings back", () => {
    // week_context_including lifts what LeftOutReason::includable allows, and that is one reason.
    expect(builders).toMatch(/reason\.includable\(\) && include\.contains/);
    const rule = aiGate.match(/fn includable\(self\) -> bool \{\s*self == Self::(\w+)\s*\}/);
    expect(snake(rule?.[1] ?? "")).toBe(INCLUDABLE_REASON);
  });

  it("marks graded-looking titles with the facade's words", () => {
    expect(ASSESSMENT_WORDS).toEqual(words("ASSESSMENT_WORDS"));
    expect(STUDY_WORDS).toEqual(words("STUDY_WORDS"));
    expect(looksLikeAssessment("Assignment 4 — Survey Simulation")).toBe(true);
    expect(looksLikeAssessment("HW3 solutions")).toBe(false);
    expect(looksLikeAssessment("hw3")).toBe(true);
    expect(looksLikeAssessment("Midterm review")).toBe(false);
  });
});
