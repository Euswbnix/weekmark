import { Archive, CalendarRange, type LucideIcon, RefreshCw } from "lucide-react";
import type { WhatsNewTopic } from "@/api/types";

/** Every topic's icon. A new WhatsNewTopic value adds its icon here, and its copy in updates.json. */
export const TOPIC_ICON: Record<WhatsNewTopic, LucideIcon> = {
  update_check: RefreshCw,
  course_weeks: CalendarRange,
  course_removal: Archive,
};

/** Every WhatsNewTopic value this build knows. */
export const WHATS_NEW_TOPICS = Object.keys(TOPIC_ICON) as WhatsNewTopic[];

/**
 * The topics this build can show: those with an icon and a title and body (in the student's
 * language or English). The rest are left out, never the whole sheet's acknowledgement.
 */
export function shownTopics(
  topics: readonly string[],
  hasCopy: (key: string) => boolean,
): WhatsNewTopic[] {
  const icons: Partial<Record<string, LucideIcon>> = TOPIC_ICON;
  return topics.filter(
    (topic): topic is WhatsNewTopic =>
      icons[topic] !== undefined &&
      hasCopy(`whatsNew.topics.${topic}.title`) &&
      hasCopy(`whatsNew.topics.${topic}.body`),
  );
}
