import React from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { Dropdown } from "../../ui/Dropdown";
import { ShortcutInput } from "../ShortcutInput";
import { useSettings } from "../../../hooks/useSettings";
import { useModelStore } from "../../../stores/modelStore";

export const SecondaryModelCard: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const { models } = useModelStore();

  const secondaryModel = getSetting("secondary_model") ?? "";
  const options = [
    { value: "", label: t("settings.secondaryModel.model.none") },
    ...models
      .filter((m) => m.is_downloaded)
      .map((m) => ({ value: m.id, label: m.name })),
  ];

  return (
    <SettingsGroup title={t("settings.secondaryModel.title")}>
      <SettingContainer
        title={t("settings.secondaryModel.model.label")}
        description={t("settings.secondaryModel.model.description")}
        descriptionMode="tooltip"
        grouped={true}
      >
        <Dropdown
          options={options}
          selectedValue={secondaryModel}
          onSelect={(value) => updateSetting("secondary_model", value)}
          disabled={isUpdating("secondary_model")}
        />
      </SettingContainer>
      {secondaryModel !== "" && (
        <ShortcutInput shortcutId="transcribe_secondary" grouped={true} />
      )}
    </SettingsGroup>
  );
};
