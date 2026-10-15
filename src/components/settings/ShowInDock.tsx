import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface ShowInDockProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const ShowInDock: React.FC<ShowInDockProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const showInDock = getSetting("show_in_dock") ?? true;
    // Without a tray icon the Dock icon is the only way back into the app,
    // so it cannot be hidden.
    const trayHidden = !(getSetting("show_tray_icon") ?? true);

    return (
      <ToggleSwitch
        checked={showInDock || trayHidden}
        onChange={(enabled) => updateSetting("show_in_dock", enabled)}
        isUpdating={isUpdating("show_in_dock")}
        disabled={trayHidden}
        label={t("settings.advanced.showInDock.label")}
        description={
          trayHidden
            ? t("settings.advanced.showInDock.requiresTray")
            : t("settings.advanced.showInDock.description")
        }
        descriptionMode={descriptionMode}
        grouped={grouped}
        tooltipPosition="bottom"
      />
    );
  },
);
