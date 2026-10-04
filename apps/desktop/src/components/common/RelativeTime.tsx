import { useTranslation } from "react-i18next";
import { formatDateTime, formatRelative } from "@/lib/format";
import { useMinute } from "@/lib/useMinute";

/**
 * "3 hours ago" with the exact date in a tooltip and the machine-readable <time>. The text
 * keeps counting while it is on screen.
 */
export function RelativeTime({ iso, now }: { iso: string; now?: Date }) {
  const { i18n } = useTranslation();
  useMinute();
  return (
    <time dateTime={iso} title={formatDateTime(iso, i18n.language)}>
      {formatRelative(iso, i18n.language, now)}
    </time>
  );
}
