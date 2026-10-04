import { RelativeTime } from "./RelativeTime";

/** Pass this as `when` to t(); <SentenceWithTime> swaps it for a <RelativeTime>. */
export const WHEN = "\u0000";
/** The same for a second time in the sentence (`iso2`). */
export const WHEN_2 = "\u0001";

const TIMES = new RegExp(`([${WHEN}${WHEN_2}])`);

/**
 * Renders a translated sentence containing {{when}} with a semantic <time> element in its
 * place, so it works in any word order ("synced 2 hours ago" / "2小时前同步").
 *
 *   <SentenceWithTime text={t("card.next", { title, when: WHEN })} iso={dueAt} />
 *
 * A sentence with two times takes the second as WHEN_2 / `iso2`.
 */
export function SentenceWithTime({
  text,
  iso,
  iso2,
}: {
  text: string;
  iso: string;
  /** The time that replaces WHEN_2, for a sentence with two. */
  iso2?: string;
}) {
  // Either time may come first, depending on the language.
  const parts = text.split(TIMES);
  return (
    <>
      {parts.map((part, index) =>
        part === WHEN ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: the parts of one sentence never reorder.
          <RelativeTime key={index} iso={iso} />
        ) : part === WHEN_2 && iso2 ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: the parts of one sentence never reorder.
          <RelativeTime key={index} iso={iso2} />
        ) : (
          part
        ),
      )}
    </>
  );
}
