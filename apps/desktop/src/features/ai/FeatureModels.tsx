import { useId } from "react";
import { useTranslation } from "react-i18next";
import {
  AI_FEATURES,
  type AiBackendStatus,
  type AiFeature,
  backendKey,
  EFFORTS,
  type Effort,
  type FeatureRouting,
  type ModelChoice,
  type ModelInfo,
} from "@/api/ai";
import { useBackendModels, useSetFeatureModel, useTestModel } from "@/api/ai-queries";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { useAiErrorText } from "./useAiErrorText";

const NONE = "none";

interface Option {
  status: AiBackendStatus;
  models: ModelInfo[];
}

/** `<backend key>|<model>`: backend keys never contain "|", so the first one splits. */
function optionValue(backendKeyValue: string, model: string): string {
  return `${backendKeyValue}|${model}`;
}

/**
 * "Which model does what": a model and an effort per feature, and a Test button (a tiny real
 * call). Only backends that are set up are offered; each model shows where it runs and whether
 * it has a price.
 */
export function FeatureModels({
  backends,
  features,
}: {
  backends: AiBackendStatus[];
  features: FeatureRouting[];
}) {
  const { t } = useTranslation("ai");
  const errorText = useAiErrorText();
  const headingId = useId();
  const usable = backends.filter((b) => b.state === "ready" || b.state === "needs_disclosure");
  const lists = useBackendModels(usable.map((b) => b.backend));
  const options: Option[] = usable.map((status, i) => ({
    status,
    models: lists[i]?.data ?? [],
  }));

  return (
    <section aria-labelledby={headingId} className="space-y-3">
      <div className="space-y-1">
        <h3 id={headingId} className="font-medium">
          {t("features.title")}
        </h3>
        <p className="text-sm text-muted-foreground">{t("features.hint")}</p>
      </div>
      {usable.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("features.turnOnFirst")}</p>
      ) : (
        <>
          <ul className="divide-y border-y">
            {AI_FEATURES.map((feature) => (
              <FeatureRow
                key={feature}
                feature={feature}
                choice={features.find((f) => f.feature === feature)?.choice ?? null}
                options={options}
              />
            ))}
          </ul>
          {usable.map((status, i) => {
            const list = lists[i];
            return list?.isError ? (
              <p key={backendKey(status.backend)} role="alert" className="text-sm text-destructive">
                {t("features.modelsFailed", { name: status.label })} {errorText(list.error)}
              </p>
            ) : null;
          })}
        </>
      )}
    </section>
  );
}

function FeatureRow({
  feature,
  choice,
  options,
}: {
  feature: AiFeature;
  choice: ModelChoice | null;
  options: Option[];
}) {
  const { t } = useTranslation("ai");
  const setModel = useSetFeatureModel();
  const test = useTestModel();
  const errorText = useAiErrorText();
  const name = t(`features.name.${feature}`);
  const resultId = useId();
  const nameId = useId();

  const value = choice ? optionValue(backendKey(choice.backend), choice.model) : NONE;
  const chosen = choice
    ? options.find((o) => backendKey(o.status.backend) === backendKey(choice.backend))
    : undefined;
  const info = chosen?.models.find((m) => m.id === choice?.model) ?? null;

  function save(next: ModelChoice | null) {
    test.reset();
    setModel.mutate({ feature, choice: next });
  }

  function onModelChange(next: string) {
    if (next === NONE) return save(null);
    for (const option of options) {
      const prefix = `${backendKey(option.status.backend)}|`;
      if (next.startsWith(prefix)) {
        return save({
          backend: option.status.backend,
          model: next.slice(prefix.length),
          effort: choice?.effort ?? "lowest",
        });
      }
    }
  }

  const notes: string[] = [];
  if (info && choice) {
    if (!info.price_known && chosen?.status.kind === "api_key") {
      notes.push(t("features.noPriceNote", { model: choice.model }));
    }
    if (info.reasoning_always_on) notes.push(t("features.thinkingNote", { model: choice.model }));
    if (info.runs_in_cloud) {
      notes.push(t("features.cloudNote", { model: choice.model }));
    }
  }

  return (
    <li className="space-y-2 py-3">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <span id={nameId} className="min-w-40 flex-1 text-sm font-medium">
          {name}
        </span>
        <Select value={value} onValueChange={onModelChange}>
          <SelectTrigger className="w-64" aria-label={t("features.model", { feature: name })}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={NONE}>{t("features.notSet")}</SelectItem>
            {options.map((option) => (
              <SelectGroup key={backendKey(option.status.backend)}>
                <SelectLabel>
                  {option.status.state === "ready"
                    ? option.status.label
                    : `${option.status.label} · ${t("backend.state.needs_disclosure")}`}
                </SelectLabel>
                {/* Keep the saved choice selectable while its model list loads. */}
                {withChosen(option, choice).map((model) => (
                  <SelectItem
                    key={model.id}
                    value={optionValue(backendKey(option.status.backend), model.id)}
                  >
                    {model.id}
                    <ModelBadges model={model} status={option.status} feature={feature} />
                  </SelectItem>
                ))}
              </SelectGroup>
            ))}
          </SelectContent>
        </Select>
        <Select
          value={choice?.effort ?? "lowest"}
          onValueChange={(effort) => choice && save({ ...choice, effort: effort as Effort })}
          disabled={!choice}
        >
          <SelectTrigger className="w-32" aria-label={t("features.effort", { feature: name })}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {EFFORTS.map((effort) => (
              <SelectItem key={effort} value={effort}>
                {t(`features.effortName.${effort}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={!choice}
          aria-describedby={`${nameId} ${resultId}`}
          onClick={() => choice && test.mutate({ backend: choice.backend, model: choice.model })}
        >
          {test.isPending ? <Spinner aria-hidden /> : null}
          {test.isPending ? t("features.testing") : t("features.test")}
        </Button>
      </div>
      {notes.map((note) => (
        <p key={note} className="text-sm text-muted-foreground">
          {note}
        </p>
      ))}
      <p id={resultId} aria-live="polite" className="text-sm">
        {test.isSuccess && test.data.ok ? (
          <span className="text-muted-foreground">
            {t("features.testOk", { seconds: ((test.data.latency_ms ?? 0) / 1000).toFixed(1) })}
          </span>
        ) : null}
        {test.isError ? <span className="text-destructive">{errorText(test.error)}</span> : null}
        {test.isSuccess && !test.data.ok && test.data.error ? (
          <span className="text-destructive">{t(`modelError.${test.data.error}`)}</span>
        ) : null}
        {setModel.isError ? (
          <span className="text-destructive">{errorText(setModel.error)}</span>
        ) : null}
      </p>
    </li>
  );
}

/** The backend's models, plus the saved choice if the list doesn't have it (yet). */
function withChosen(option: Option, choice: ModelChoice | null): ModelInfo[] {
  if (
    !choice ||
    backendKey(choice.backend) !== backendKey(option.status.backend) ||
    option.models.some((m) => m.id === choice.model)
  ) {
    return option.models;
  }
  return [
    ...option.models,
    {
      id: choice.model,
      on_device: option.status.kind === "local",
      runs_in_cloud: false,
      price_known: true,
      reasoning_always_on: false,
      suggested_for: [],
    },
  ];
}

function ModelBadges({
  model,
  status,
  feature,
}: {
  model: ModelInfo;
  status: AiBackendStatus;
  feature: AiFeature;
}) {
  const { t } = useTranslation("ai");
  const badges: string[] = [];
  if (model.runs_in_cloud) badges.push(t("features.badge.cloud"));
  else if (model.on_device) badges.push(t("features.badge.onDevice"));
  if (status.kind === "api_key" && !model.price_known) badges.push(t("features.badge.noPrice"));
  if (model.suggested_for.includes(feature)) badges.push(t("features.badge.suggested"));
  if (badges.length === 0) return null;
  return <span className="text-xs text-muted-foreground">({badges.join(", ")})</span>;
}
