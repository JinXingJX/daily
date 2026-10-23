import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import type {
  SettingsScope,
  SettingsView
} from "@/types";
import {
  Save
} from "lucide-react";
import { type Dispatch, type SetStateAction } from "react";
import { SettingsHeader } from "./settingsShared";
import { currentLocale, t } from "@/i18n";
import { localizeHostText } from "@daily/plugin-sdk/hostText";

export function ConfigFileSettings({
  settings,
  selectedScope,
  scopeDrafts,
  setSelectedScope,
  setScopeDrafts,
  onScopeSave,
}: {
  settings: SettingsView;
  selectedScope: SettingsScope;
  scopeDrafts: Record<SettingsScope, string>;
  setSelectedScope: (scope: SettingsScope) => void;
  setScopeDrafts: Dispatch<SetStateAction<Record<SettingsScope, string>>>;
  onScopeSave: () => void;
}) {
  const selected = settings.scopes.find((scope) => scope.scope === selectedScope);

  return (
    <section>
      <SettingsHeader title={t("Settings files")} detail={t("Precedence: local > project > user. More specific scopes override broader ones. Provider configurations (endpoints and keys) are read from user and local settings only.")} />
      {settings.warnings.map((warning) => (
        <p key={warning} role="status" className="mb-3 mt-0 rounded-[var(--radius-control)] bg-[var(--warning-soft)] px-3 py-2 text-xs leading-5 text-[var(--warning)]">
          {localizeHostText(warning, currentLocale())}
        </p>
      ))}
      <div className="settings-group settings-form">
        <div className="mb-4 flex flex-wrap gap-2">
          {settings.scopes.map((scope) => (
            <Button
              key={scope.scope}
              variant={scope.scope === selectedScope ? "secondary" : "ghost"}
              onClick={() => setSelectedScope(scope.scope)}
            >
              {scope.scope}
            </Button>
          ))}
        </div>
        <div className="mb-3 text-xs text-[var(--text-muted)]">{selected?.path ?? t("No file selected")}</div>
        <Textarea
          className="min-h-[28rem] resize-y font-mono text-sm"
          value={scopeDrafts[selectedScope] ?? ""}
          onChange={(event) => {
            const content = event.currentTarget.value;
            setScopeDrafts((current) => ({
              ...current,
              [selectedScope]: content,
            }));
          }}
        />
        <div className="mt-4 flex justify-end">
          <Button disabled={!selected?.editable} onClick={onScopeSave}>
            <Save className="size-4" />
            {t("Save file")}
          </Button>
        </div>
      </div>
    </section>
  );
}
