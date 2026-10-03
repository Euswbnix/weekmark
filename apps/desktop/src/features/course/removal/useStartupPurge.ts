import { useEffect, useRef } from "react";
import { useStartupTasks } from "@/api/queries";
import { usePurgeRemovedCourses } from "@/api/removalQueries";
import { REMOVAL_UI } from "./availability";

/**
 * The app-start purge (calendar design §8.3): when the facade says removed courses wait for it
 * (`startup_tasks.purge_due`: due, or a Trash move left files), run it once per launch. It goes
 * through the install gate like any work; a failure leaves it due for the next launch. Mount
 * once, in the app shell.
 */
export function useStartupPurge(enabled: boolean = REMOVAL_UI) {
  const tasks = useStartupTasks();
  const purge = usePurgeRemovedCourses();
  const started = useRef(false);
  const due = tasks.data?.purge_due === true;
  const { mutate } = purge;
  useEffect(() => {
    if (!enabled || !due || started.current) return;
    started.current = true;
    mutate({ removedIds: null, permanentIfNoTrash: false });
  }, [enabled, due, mutate]);
}
