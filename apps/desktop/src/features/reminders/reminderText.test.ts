import { describe, expect, it } from "vitest";
import type { Reminder } from "@/api/reminders";
import i18n from "@/i18n";
import { reminderText } from "./reminderText";

const base = {
  local_time: "2026-11-02T09:00",
  time_zone: "America/Toronto",
  fire_at: "2026-11-02T14:00:00Z",
};

/** ICU puts a narrow no-break space before AM/PM in some versions and a space in others. */
function text(reminder: Reminder, locale = "en") {
  const out = reminderText(reminder, i18n.getFixedT(locale, "reminders"), locale);
  return { ...out, body: out.body.replace(/\s/g, " ") };
}

describe("reminder notifications", () => {
  it("name a deadline by course code and title, due in the reminder's time zone", () => {
    const deadline: Reminder = {
      ...base,
      id: "deadline_soon:a2:24",
      kind: "deadline_soon",
      title: "Assignment 2",
      course_code: "DEMO101",
      course_name: "Intro to Demo Studies",
      // 23:59 in Toronto on Tuesday 3 November 2026 (EST, after DST ended on 1 November).
      due_at: "2026-11-04T04:59:00Z",
      hours_before: 24,
    };
    expect(text(deadline)).toEqual({
      id: "deadline_soon:a2:24",
      title: "DEMO101: Assignment 2",
      body: "Due Tuesday 11:59 PM",
    });
    // The day and the time apart (CLDR's zh pattern runs them together).
    expect(text(deadline, "zh-CN")).toMatchObject({
      title: "DEMO101：Assignment 2",
      body: "截止时间：星期二 23:59",
    });
  });

  it("use the course name without a code, and only the title for an unlinked feed event", () => {
    const deadline: Reminder = {
      ...base,
      id: "d",
      kind: "deadline_soon",
      title: "Lab report",
      course_name: "Demo Lab",
      due_at: "2026-11-04T04:59:00Z",
    };
    expect(text(deadline).title).toBe("Demo Lab: Lab report");
    expect(text({ ...deadline, course_name: null }).title).toBe("Lab report");
  });

  it("count the week's deadlines and today's plan items", () => {
    const digest: Reminder = { ...base, id: "w", kind: "weekly_digest", count: 2 };
    expect(text(digest)).toMatchObject({
      title: "Your week in PageLamp",
      body: "2 deadlines in the next 7 days",
    });
    expect(text({ ...digest, count: 1 }).body).toBe("1 deadline in the next 7 days");
    expect(text({ ...digest, count: 0 }).body).toBe("No deadlines in the next 7 days");
    const plan: Reminder = { ...base, id: "p", kind: "plan_today", count: 3 };
    expect(text(plan)).toMatchObject({
      title: "Today's study plan",
      body: "3 things to do today",
    });
    expect(text(plan, "zh-CN").body).toBe("今天有 3 件事要做");
  });

  it("fall back to this system's zone for a time zone it doesn't know", () => {
    const deadline: Reminder = {
      ...base,
      id: "d",
      kind: "deadline_soon",
      title: "Quiz",
      course_code: "DEMO101",
      time_zone: "Not/AZone",
      due_at: "2026-11-04T04:59:00Z",
    };
    expect(text(deadline).body).toMatch(/^Due \w+day \d{1,2}:\d{2} [AP]M$/);
  });
});
