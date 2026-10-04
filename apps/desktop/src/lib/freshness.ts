/**
 * A source has two clocks once PageLamp syncs by itself: its last FULL sync (modules, materials,
 * everything) and, when a lighter automatic sync has run since, the time its deadlines and
 * announcements were last read. Returns that second time only when it is the later one, so a
 * line can say both without ever calling the materials fresher than they are.
 */
export function deadlinesReadSince(
  full: string | null | undefined,
  deadlines: string | null | undefined,
): string | null {
  if (!deadlines) return null;
  return !full || Date.parse(deadlines) > Date.parse(full) ? deadlines : null;
}
