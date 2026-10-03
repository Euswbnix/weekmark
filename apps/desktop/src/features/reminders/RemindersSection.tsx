import { type ReactNode, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { useApi } from "@/api/context";
import { useUpdaterStatus } from "@/api/queries";
import type { BackgroundStatus, ReminderSettings } from "@/api/reminders";
import { WEEKDAYS, type Weekday } from "@/api/reminders";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { SettingsSection } from "@/features/settings/SettingsSection";
import { settingsSections } from "@/lib/routes";
import { useBackgroundStatus, useReminderSettings, useSetReminderSettings } from "./queries";

/**
 * Settings → Reminders (design §5.3, §7): "Keep PageLamp in the tray and start it at login" (the
 * onboarding answer, and the student's consent to notifications), then the kinds and their times.
 */
export function RemindersSection() {
  const { t } = useTranslation("reminders");
  const settings = useReminderSettings();
  const background = useBackgroundStatus();
  return (
    <SettingsSection
      id={settingsSections.reminders}
      title={t("settings.title")}
      description={t("settings.description")}
    >
      {settings.data ? (
        <RemindersForm settings={settings.data} background={background.data ?? null} />
      ) : null}
    </SettingsSection>
  );
}

function RemindersForm({
  settings,
  background,
}: {
  settings: ReminderSettings;
  background: BackgroundStatus | null;
}) {
  const { t } = useTranslation("reminders");
  const save = useSetReminderSettings();
  const set = (patch: Partial<ReminderSettings>) => save.mutate({ ...settings, ...patch });
  const on = settings.run_in_background;
  // Only a status that answers the current setting: while turning it on, the old one would say
  // the login item is missing.
  const status = on && background?.run_in_background ? background : null;
  const note = status?.tray_unavailable
    ? t("settings.trayUnavailable")
    : status && !status.login_item
      ? t("settings.loginItemOff")
      : null;

  return (
    <div className="space-y-5">
      <SwitchRow
        checked={on}
        label={t("settings.background")}
        hint={t("settings.backgroundHint")}
        onChange={(checked) => set({ run_in_background: checked })}
      >
        {note ? <p className="text-xs text-muted-foreground">{note}</p> : null}
        {on ? <NotSeeingReminders /> : null}
      </SwitchRow>

      {on ? (
        <div className="divide-y border-y">
          <div className="py-4">
            <SwitchRow
              checked={settings.deadline_soon}
              label={t("settings.deadlineSoon")}
              hint={t("settings.deadlineSoonHint")}
              onChange={(checked) => set({ deadline_soon: checked })}
            />
          </div>
          <div className="py-4">
            <SwitchRow
              checked={settings.weekly_digest}
              label={t("settings.weeklyDigest")}
              hint={t("settings.weeklyDigestHint")}
              onChange={(checked) => set({ weekly_digest: checked })}
            >
              {settings.weekly_digest ? (
                <div className="flex flex-wrap items-end gap-3 pt-2">
                  <DayField
                    context={t("settings.weeklyDigest")}
                    value={settings.digest_day}
                    onChange={(day) => set({ digest_day: day })}
                  />
                  <TimeField
                    context={t("settings.weeklyDigest")}
                    value={settings.digest_time}
                    onChange={(time) => set({ digest_time: time })}
                  />
                </div>
              ) : null}
            </SwitchRow>
          </div>
          <div className="py-4">
            <SwitchRow
              checked={settings.plan_today}
              label={t("settings.planToday")}
              hint={t("settings.planTodayHint")}
              onChange={(checked) => set({ plan_today: checked })}
            >
              {settings.plan_today ? (
                <div className="pt-2">
                  <TimeField
                    context={t("settings.planToday")}
                    value={settings.plan_today_time}
                    onChange={(time) => set({ plan_today_time: time })}
                  />
                </div>
              ) : null}
            </SwitchRow>
          </div>
        </div>
      ) : null}

      {save.isError ? (
        <p role="alert" className="text-sm text-destructive">
          {t("settings.saveFailed")}
        </p>
      ) : null}
    </div>
  );
}

/**
 * Whether the system lets PageLamp notify can't be read on desktop (the plugin always says yes),
 * so say where to look, with the settings pane where there's a standard one (macOS, Windows).
 */
function NotSeeingReminders() {
  const { t } = useTranslation("reminders");
  const api = useApi();
  const platform = useUpdaterStatus().data?.platform;
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 pt-1">
      <p className="text-xs text-muted-foreground">{t("settings.notSeeing")}</p>
      {platform === "macos" || platform === "windows" ? (
        <Button
          type="button"
          size="sm"
          variant="link"
          className="h-auto p-0 text-xs"
          onClick={() => void api.openNotificationSettings().catch(() => false)}
        >
          {t("settings.openNotificationSettings")}
        </Button>
      ) : null}
    </div>
  );
}

function SwitchRow({
  checked,
  label,
  hint,
  onChange,
  children,
}: {
  checked: boolean;
  label: string;
  hint: string;
  onChange: (checked: boolean) => void;
  children?: ReactNode;
}) {
  const id = useId();
  const hintId = useId();
  return (
    <div className="flex items-start gap-3">
      <Switch id={id} checked={checked} aria-describedby={hintId} onCheckedChange={onChange} />
      <div className="min-w-0 flex-1 space-y-0.5">
        <Label htmlFor={id}>{label}</Label>
        <p id={hintId} className="text-xs text-muted-foreground">
          {hint}
        </p>
        {children}
      </div>
    </div>
  );
}

/** Weekday names come from the system, in the student's language (no strings to translate). */
function DayField({
  context,
  value,
  onChange,
}: {
  /** The reminder it belongs to, in the accessible name ("Day for Your week"). */
  context: string;
  value: Weekday;
  onChange: (day: Weekday) => void;
}) {
  const { t, i18n } = useTranslation("reminders");
  const id = useId();
  const name = (index: number) =>
    new Intl.DateTimeFormat(i18n.language, { weekday: "long", timeZone: "UTC" }).format(
      // 2024-01-01 was a Monday.
      Date.UTC(2024, 0, 1 + index),
    );
  return (
    <div className="space-y-1">
      <Label htmlFor={id} className="text-xs">
        {t("settings.day")}
      </Label>
      <Select
        value={value}
        onValueChange={(day) => {
          if ((WEEKDAYS as readonly string[]).includes(day)) onChange(day as Weekday);
        }}
      >
        <SelectTrigger
          id={id}
          aria-label={t("settings.fieldOf", { field: t("settings.day"), item: context })}
          className="w-40"
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {WEEKDAYS.map((day, index) => (
            <SelectItem key={day} value={day}>
              {name(index)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

const HH_MM = /^([01]\d|2[0-3]):[0-5]\d$/;

/** "HH:MM", saved when the field is left with a valid time. */
function TimeField({
  context,
  value,
  onChange,
}: {
  /** As in DayField: "Time for Your week" and "Time for Today's study plan", not two "Time"s. */
  context: string;
  value: string;
  onChange: (time: string) => void;
}) {
  const { t } = useTranslation("reminders");
  const id = useId();
  const errorId = useId();
  const [draft, setDraft] = useState(value);
  const invalid = !HH_MM.test(draft);
  return (
    <div className="space-y-1">
      <Label htmlFor={id} className="text-xs">
        {t("settings.time")}
      </Label>
      <Input
        id={id}
        type="time"
        aria-label={t("settings.fieldOf", { field: t("settings.time"), item: context })}
        value={draft}
        className="w-32"
        aria-invalid={invalid || undefined}
        aria-describedby={invalid ? errorId : undefined}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={() => {
          if (!invalid && draft !== value) onChange(draft);
        }}
      />
      {invalid ? (
        <p id={errorId} className="text-xs text-destructive">
          {t("settings.invalidTime")}
        </p>
      ) : null}
    </div>
  );
}
