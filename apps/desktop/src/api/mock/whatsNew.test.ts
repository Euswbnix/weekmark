import { describe, expect, it } from "vitest";
import { WHATS_NEW_TOPICS } from "@/features/updates/whatsNewTopics";
import { compareVersions, topicsSince, WHATS_NEW } from "./whatsNew";

describe("mock What's new", () => {
  it("orders versions like semver, pre-releases first", () => {
    const sorted = ["0.3.0", "0.3.0-beta.1", "0.3.0-alpha.10", "0.3.0-alpha.2", "0.2.9"].sort(
      compareVersions,
    );
    expect(sorted).toEqual(["0.2.9", "0.3.0-alpha.2", "0.3.0-alpha.10", "0.3.0-beta.1", "0.3.0"]);
  });

  it("gives an upgrader the topics after their version, and one from 0.1 everything", () => {
    expect(topicsSince("0.3.0-alpha.1")).toEqual([
      "course_removal",
      "syllabus_reading",
      "ai_writing",
      "reminders",
    ]);
    expect(topicsSince("0.3.0-alpha.2")).toEqual(["syllabus_reading", "ai_writing", "reminders"]);
    expect(topicsSince("0.3.0-alpha.3")).toEqual(["ai_writing", "reminders"]);
    expect(topicsSince("0.3.0-beta.1")).toEqual([]);
    expect(topicsSince(null)).toEqual([
      "update_check",
      "course_weeks",
      "course_removal",
      "syllabus_reading",
      "ai_writing",
      "reminders",
    ]);
  });

  it("mirrors the facade: one row per WhatsNewTopic", () => {
    expect(WHATS_NEW.map(([topic]) => topic).sort()).toEqual([...WHATS_NEW_TOPICS].sort());
  });
});
