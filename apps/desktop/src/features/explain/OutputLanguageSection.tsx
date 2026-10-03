import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { OutputLanguage } from "@/api/explain";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { SettingsSection } from "@/features/settings/SettingsSection";
import { useOutputLanguage, useSetOutputLanguage } from "./useExplanation";

const LANGUAGES: OutputLanguage[] = ["ui", "course"];

/** Settings → AI → the language of explanations (design §7): PageLamp's, or the course's. */
export function OutputLanguageSection() {
  const { t } = useTranslation("explain");
  const language = useOutputLanguage();
  const save = useSetOutputLanguage();
  const labelId = useId();
  return (
    <SettingsSection title={t("language.title")} description={t("language.description")}>
      {language.data ? (
        <RadioGroup
          aria-labelledby={labelId}
          value={language.data}
          onValueChange={(value) => save.mutate(value as OutputLanguage)}
          className="gap-3"
        >
          <span id={labelId} className="sr-only">
            {t("language.title")}
          </span>
          {LANGUAGES.map((value) => (
            <LanguageOption key={value} value={value} />
          ))}
        </RadioGroup>
      ) : null}
    </SettingsSection>
  );
}

function LanguageOption({ value }: { value: OutputLanguage }) {
  const { t } = useTranslation("explain");
  const id = useId();
  return (
    <div className="flex items-center gap-2">
      <RadioGroupItem id={id} value={value} />
      <Label htmlFor={id}>{t(`language.${value}`)}</Label>
    </div>
  );
}
