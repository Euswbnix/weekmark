import { describe, expect, it } from "vitest";
import i18n from "@/i18n";
import { shownTopics, WHATS_NEW_TOPICS } from "./whatsNewTopics";

describe("What's new topics", () => {
  it("has an icon and its own en and zh-CN copy for every WhatsNewTopic", () => {
    for (const locale of ["en", "zh-CN"]) {
      for (const topic of WHATS_NEW_TOPICS) {
        for (const part of ["title", "body"]) {
          const copy = i18n.getResource(locale, "updates", `whatsNew.topics.${topic}.${part}`);
          expect(copy, `${locale} ${topic}.${part}`).toEqual(expect.any(String));
          expect(copy).not.toBe("");
        }
      }
    }
  });

  it("leaves out a topic this build has no icon or copy for, keeping the order", () => {
    const hasCopy = (key: string) => !key.includes("course_weeks");
    expect(shownTopics(["update_check", "not_a_topic", "course_weeks"], hasCopy)).toEqual([
      "update_check",
    ]);
    expect(shownTopics(["course_weeks", "update_check"], () => true)).toEqual([
      "course_weeks",
      "update_check",
    ]);
  });
});
