import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { useApi } from "@/api/context";
import { queryKeys } from "@/api/queries";
import type { ReminderSettings } from "@/api/reminders";

export const reminderKeys = {
  settings: () => [...queryKeys.all, "reminders", "settings"] as const,
  background: () => [...queryKeys.all, "reminders", "background"] as const,
};

export function useReminderSettings() {
  const api = useApi();
  return useQuery({ queryKey: reminderKeys.settings(), queryFn: () => api.reminderSettings() });
}

/** The tray and the login item as they are now (the student may change the latter elsewhere). */
export function useBackgroundStatus() {
  const api = useApi();
  return useQuery({ queryKey: reminderKeys.background(), queryFn: () => api.backgroundStatus() });
}

/**
 * Saves the reminder settings (optimistically, so quick changes build on each other); the shell
 * follows run_in_background. Turning it on sends the one "Reminders are on" notification: where
 * the system asks whether PageLamp may notify.
 */
export function useSetReminderSettings() {
  const api = useApi();
  const client = useQueryClient();
  const { t } = useTranslation("reminders");
  return useMutation({
    // One save after another, in the order made (the shell applies them one at a time too).
    scope: { id: "reminder-settings" },
    mutationFn: (settings: ReminderSettings) => api.setReminderSettings(settings),
    onMutate: async (settings) => {
      await client.cancelQueries({ queryKey: reminderKeys.settings() });
      const previous = client.getQueryData<ReminderSettings>(reminderKeys.settings());
      client.setQueryData(reminderKeys.settings(), settings);
      return { previous };
    },
    onSuccess: (background, settings, context) => {
      client.setQueryData(reminderKeys.background(), background);
      if (settings.run_in_background && !context?.previous?.run_in_background) {
        api.showRemindersOnNotice(t("notify.onTitle"), t("notify.onBody")).catch(() => {});
      }
    },
    onError: (_error, _settings, context) => {
      client.setQueryData(reminderKeys.settings(), context?.previous);
    },
  });
}
