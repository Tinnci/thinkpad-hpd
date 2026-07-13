import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM

KCM.SimpleKCM {
    Kirigami.FormLayout {
        anchors.fill: parent

        QQC2.Switch { Kirigami.FormData.label: i18n("Human presence:"); text: i18n("Enable presence automation"); checked: kcm.enabled; onToggled: { kcm.enabled = checked; kcm.changed() } }
        Kirigami.InlineMessage { Kirigami.FormData.isSection: true; visible: kcm.operationError.length > 0; type: Kirigami.MessageType.Error; text: kcm.operationError }
        QQC2.CheckBox { text: i18n("Simulation only (do not control the desktop)"); enabled: kcm.enabled; checked: kcm.dryRun; onToggled: { kcm.dryRun = checked; kcm.changed() } }
        Kirigami.InlineMessage { Kirigami.FormData.isSection: true; visible: true; type: kcm.screenOffSupported ? Kirigami.MessageType.Information : Kirigami.MessageType.Warning; text: kcm.diagnosticSummary }
        Kirigami.InlineMessage { Kirigami.FormData.isSection: true; visible: true; type: kcm.serviceReady ? (kcm.enabled ? Kirigami.MessageType.Positive : Kirigami.MessageType.Information) : Kirigami.MessageType.Warning; text: kcm.serviceSummary }

        Kirigami.Separator { Kirigami.FormData.isSection: true }
        Kirigami.Heading { Kirigami.FormData.isSection: true; level: 3; text: i18n("Away behavior") }
        QQC2.CheckBox { text: i18n("Lock screen automatically"); enabled: kcm.enabled; checked: kcm.lockScreen; onToggled: { kcm.lockScreen = checked; kcm.changed() } }
        QQC2.SpinBox { Kirigami.FormData.label: i18n("Away confirmation (seconds):"); enabled: kcm.enabled && kcm.lockScreen; from: 1; to: 3600; value: kcm.awaySeconds; onValueModified: { kcm.awaySeconds = value; kcm.changed() } }
        QQC2.SpinBox { Kirigami.FormData.label: i18n("Input idle time (seconds):"); enabled: kcm.enabled && kcm.lockScreen; from: 0; to: 3600; value: kcm.idleSeconds; onValueModified: { kcm.idleSeconds = value; kcm.changed() } }
        QQC2.SpinBox { Kirigami.FormData.label: i18n("Startup grace (seconds):"); enabled: kcm.enabled && kcm.lockScreen; from: 0; to: 600; value: kcm.startupGraceSeconds; onValueModified: { kcm.startupGraceSeconds = value; kcm.changed() } }
        QQC2.CheckBox { text: i18n("Turn off display after locking"); enabled: kcm.enabled && kcm.lockScreen && kcm.screenOffSupported; checked: kcm.turnOffScreen && kcm.screenOffSupported; onToggled: { kcm.turnOffScreen = checked; kcm.changed() } }
        QQC2.SpinBox { Kirigami.FormData.label: i18n("Display-off delay (ms):"); enabled: kcm.enabled && kcm.lockScreen && kcm.turnOffScreen; from: 0; to: 60000; stepSize: 50; value: kcm.screenOffDelayMilliseconds; onValueModified: { kcm.screenOffDelayMilliseconds = value; kcm.changed() } }

        Kirigami.Separator { Kirigami.FormData.isSection: true }
        Kirigami.Heading { Kirigami.FormData.isSection: true; level: 3; text: i18n("Return behavior") }
        QQC2.CheckBox { text: i18n("Wake lock screen when presence returns"); enabled: kcm.enabled; checked: kcm.wakeScreen; onToggled: { kcm.wakeScreen = checked; kcm.changed() } }
        QQC2.CheckBox { text: i18n("Also wake screens locked manually"); enabled: kcm.enabled && kcm.wakeScreen; checked: kcm.wakeManualLock; onToggled: { kcm.wakeManualLock = checked; kcm.changed() } }
        QQC2.SpinBox { Kirigami.FormData.label: i18n("Return confirmation (ms):"); enabled: kcm.enabled && kcm.wakeScreen; from: 100; to: 60000; stepSize: 50; value: kcm.presentMilliseconds; onValueModified: { kcm.presentMilliseconds = value; kcm.changed() } }

        Kirigami.Separator { Kirigami.FormData.isSection: true }
        Kirigami.Heading { Kirigami.FormData.isSection: true; level: 3; text: i18n("On-screen status") }
        QQC2.CheckBox { text: i18n("Show presence status"); enabled: kcm.enabled; checked: kcm.showOsd; onToggled: { kcm.showOsd = checked; kcm.changed() } }
        QQC2.SpinBox { Kirigami.FormData.label: i18n("Status delay (ms):"); from: 100; to: 60000; stepSize: 50; value: kcm.osdMilliseconds; enabled: kcm.enabled && kcm.showOsd; onValueModified: { kcm.osdMilliseconds = value; kcm.changed() } }
        QQC2.SpinBox { Kirigami.FormData.label: i18n("Minimum interval (seconds):"); from: 0; to: 300; value: kcm.osdCooldownSeconds; enabled: kcm.enabled && kcm.showOsd; onValueModified: { kcm.osdCooldownSeconds = value; kcm.changed() } }
        QQC2.TextField { Kirigami.FormData.label: i18n("Present message:"); text: kcm.presentText; maximumLength: 120; enabled: kcm.enabled && kcm.showOsd; onTextEdited: { kcm.presentText = text; kcm.changed() } }
        QQC2.TextField { Kirigami.FormData.label: i18n("Away message:"); text: kcm.awayText; maximumLength: 120; enabled: kcm.enabled && kcm.showOsd; onTextEdited: { kcm.awayText = text; kcm.changed() } }
    }
}
