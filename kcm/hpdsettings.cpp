#include <KQuickConfigModule>
#include <KPluginFactory>
#include <KLocalizedString>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcess>
#include <QDebug>

class HpdSettings final : public KQuickConfigModule
{
    Q_OBJECT
    Q_PROPERTY(bool enabled MEMBER m_enabled NOTIFY settingsChanged)
    Q_PROPERTY(bool dryRun MEMBER m_dryRun NOTIFY settingsChanged)
    Q_PROPERTY(bool lockScreen MEMBER m_lockScreen NOTIFY settingsChanged)
    Q_PROPERTY(int awaySeconds MEMBER m_away NOTIFY settingsChanged)
    Q_PROPERTY(int idleSeconds MEMBER m_idle NOTIFY settingsChanged)
    Q_PROPERTY(int startupGraceSeconds MEMBER m_startupGrace NOTIFY settingsChanged)
    Q_PROPERTY(int presentMilliseconds MEMBER m_present NOTIFY settingsChanged)
    Q_PROPERTY(int osdMilliseconds MEMBER m_osdDelay NOTIFY settingsChanged)
    Q_PROPERTY(bool turnOffScreen MEMBER m_turnOff NOTIFY settingsChanged)
    Q_PROPERTY(int screenOffDelayMilliseconds MEMBER m_screenOffDelay NOTIFY settingsChanged)
    Q_PROPERTY(bool wakeScreen MEMBER m_wake NOTIFY settingsChanged)
    Q_PROPERTY(bool wakeManualLock MEMBER m_wakeManualLock NOTIFY settingsChanged)
    Q_PROPERTY(bool showOsd MEMBER m_showOsd NOTIFY settingsChanged)
    Q_PROPERTY(int osdCooldownSeconds MEMBER m_osdCooldown NOTIFY settingsChanged)
    Q_PROPERTY(QString presentText MEMBER m_presentText NOTIFY settingsChanged)
    Q_PROPERTY(QString awayText MEMBER m_awayText NOTIFY settingsChanged)
    Q_PROPERTY(bool screenOffSupported MEMBER m_screenOffSupported NOTIFY settingsChanged)
    Q_PROPERTY(QString diagnosticSummary MEMBER m_diagnosticSummary NOTIFY settingsChanged)
    Q_PROPERTY(bool serviceReady MEMBER m_serviceReady NOTIFY settingsChanged)
    Q_PROPERTY(QString serviceSummary MEMBER m_serviceSummary NOTIFY settingsChanged)
    Q_PROPERTY(QString operationError MEMBER m_operationError NOTIFY settingsChanged)

public:
    HpdSettings(QObject *parent, const KPluginMetaData &data)
        : KQuickConfigModule(parent, data)
    {
        KLocalizedString::setApplicationDomain("kcm_thinkpadhpd");
        setButtons(Apply | Default | Help);
    }

    void load() override
    {
        m_operationError.clear();
        QByteArray output;
        QString commandError;
        if (!runCommand(QStringLiteral("thinkpad-hpd"), {QStringLiteral("settings"), QStringLiteral("get")}, &output, &commandError)) {
            m_operationError = i18n("Could not read HPD settings: %1", commandError);
        }
        QJsonParseError parseError;
        const auto document = QJsonDocument::fromJson(output, &parseError);
        if (!output.isEmpty() && (parseError.error != QJsonParseError::NoError || !document.isObject())) {
            m_operationError = i18n("Could not read HPD settings: %1", parseError.errorString());
        }
        const auto object = document.object();
        m_enabled = object[QStringLiteral("enabled")].toBool(true);
        m_dryRun = object[QStringLiteral("dry_run")].toBool(true);
        m_lockScreen = object[QStringLiteral("lock_screen")].toBool(true);
        m_away = object[QStringLiteral("away_confirm_seconds")].toInt(15);
        m_idle = object[QStringLiteral("idle_confirm_seconds")].toInt(15);
        m_startupGrace = object[QStringLiteral("startup_grace_seconds")].toInt(10);
        m_present = object[QStringLiteral("present_confirm_milliseconds")].toInt(750);
        m_osdDelay = object[QStringLiteral("osd_confirm_milliseconds")].toInt(1000);
        m_turnOff = object[QStringLiteral("turn_off_screen")].toBool(false);
        m_screenOffDelay = object[QStringLiteral("screen_off_delay_milliseconds")].toInt(750);
        m_wake = object[QStringLiteral("wake_screen")].toBool(true);
        m_wakeManualLock = object[QStringLiteral("wake_manual_lock")].toBool(false);
        m_showOsd = object[QStringLiteral("show_osd")].toBool(true);
        m_osdCooldown = object[QStringLiteral("osd_cooldown_seconds")].toInt(5);
        m_presentText = object[QStringLiteral("osd_present_text")].toString(QStringLiteral("HPD: 检测到用户"));
        m_awayText = object[QStringLiteral("osd_away_text")].toString(QStringLiteral("HPD: 用户已离开"));
        QByteArray diagnosticOutput;
        if (!runCommand(QStringLiteral("thinkpad-hpd"), {QStringLiteral("diagnose")}, &diagnosticOutput, &commandError)) {
            m_operationError = i18n("Could not run HPD diagnostics: %1", commandError);
        }
        const auto diagnosticObject = QJsonDocument::fromJson(diagnosticOutput).object();
        m_screenOffSupported = diagnosticObject[QStringLiteral("screen_off_supported")].toBool(false);
        const auto reason = diagnosticObject[QStringLiteral("screen_off_block_reason")].toString();
        const auto sensorAvailable = diagnosticObject[QStringLiteral("sensor")].toObject()[QStringLiteral("available")].toBool(false);
        m_diagnosticSummary = sensorAvailable ? i18n("Presence sensor detected") : i18n("Presence sensor unavailable");
        if (!reason.isEmpty()) {
            m_diagnosticSummary += QStringLiteral(". ")
                + i18n("Automatic display power-off is blocked on AMDGPU Wayland after observed display failures.");
        }
        QByteArray daemonEnabledOutput;
        const bool daemonActive = runCommand(QStringLiteral("systemctl"), {QStringLiteral("is-active"), QStringLiteral("thinkpad-hpd.service")}, nullptr, nullptr);
        runCommand(QStringLiteral("systemctl"), {QStringLiteral("is-enabled"), QStringLiteral("thinkpad-hpd.service")}, &daemonEnabledOutput, nullptr, true);
        const bool daemonMasked = QString::fromUtf8(daemonEnabledOutput).trimmed() == QStringLiteral("masked");
        const bool agentActive = runCommand(QStringLiteral("systemctl"), {QStringLiteral("--user"), QStringLiteral("is-active"), QStringLiteral("thinkpad-hpd-agent.service")}, nullptr, nullptr);
        m_serviceReady = daemonActive && (!m_enabled || agentActive);
        const QString daemonText = daemonMasked ? i18n("masked") : (daemonActive ? i18n("active") : i18n("inactive"));
        const QString agentText = agentActive ? i18n("active") : i18n("inactive");
        m_serviceSummary = i18n("Sensor service: %1; desktop agent: %2", daemonText, agentText);
        setNeedsSave(false);
        Q_EMIT settingsChanged();
    }

    void save() override
    {
        QJsonObject object{{QStringLiteral("enabled"), m_enabled}, {QStringLiteral("dry_run"), m_dryRun},
            {QStringLiteral("lock_screen"), m_lockScreen},
            {QStringLiteral("away_confirm_seconds"), m_away}, {QStringLiteral("idle_confirm_seconds"), m_idle},
            {QStringLiteral("startup_grace_seconds"), m_startupGrace},
            {QStringLiteral("present_confirm_milliseconds"), m_present}, {QStringLiteral("osd_confirm_milliseconds"), m_osdDelay},
            {QStringLiteral("turn_off_screen"), m_turnOff}, {QStringLiteral("screen_off_delay_milliseconds"), m_screenOffDelay},
            {QStringLiteral("wake_screen"), m_wake}, {QStringLiteral("wake_manual_lock"), m_wakeManualLock},
            {QStringLiteral("show_osd"), m_showOsd}, {QStringLiteral("osd_cooldown_seconds"), m_osdCooldown},
            {QStringLiteral("osd_present_text"), m_presentText}, {QStringLiteral("osd_away_text"), m_awayText}};
        m_operationError.clear();
        QString commandError;
        if (runCommand(QStringLiteral("thinkpad-hpd"), {QStringLiteral("settings"), QStringLiteral("set"),
            QStringLiteral("--json"), QString::fromUtf8(QJsonDocument(object).toJson(QJsonDocument::Compact))}, nullptr, &commandError)) {
            const QStringList serviceArguments = m_enabled
                ? QStringList{QStringLiteral("--user"), QStringLiteral("enable"), QStringLiteral("--now"), QStringLiteral("thinkpad-hpd-agent.service")}
                : QStringList{QStringLiteral("--user"), QStringLiteral("disable"), QStringLiteral("--now"), QStringLiteral("thinkpad-hpd-agent.service")};
            if (runCommand(QStringLiteral("systemctl"), serviceArguments, nullptr, &commandError)) {
                setNeedsSave(false);
                load();
            } else {
                m_operationError = i18n("Could not update the HPD desktop agent: %1", commandError);
                qWarning() << m_operationError;
                setNeedsSave(true);
                Q_EMIT settingsChanged();
            }
        } else {
            m_operationError = i18n("Could not save HPD settings: %1", commandError);
            qWarning() << m_operationError;
            setNeedsSave(true);
            Q_EMIT settingsChanged();
        }
    }

    void defaults() override
    {
        m_enabled = true; m_dryRun = true; m_lockScreen = true; m_away = 15; m_idle = 15; m_startupGrace = 10;
        m_present = 750; m_osdDelay = 1000; m_osdCooldown = 5; m_screenOffDelay = 750;
        m_turnOff = false; m_wake = true; m_wakeManualLock = false; m_showOsd = true;
        m_presentText = QStringLiteral("HPD: 检测到用户"); m_awayText = QStringLiteral("HPD: 用户已离开");
        setNeedsSave(true); Q_EMIT settingsChanged();
    }

    Q_INVOKABLE void changed() { setNeedsSave(true); }
Q_SIGNALS:
    void settingsChanged();
private:
    static bool runCommand(const QString &program, const QStringList &arguments, QByteArray *standardOutput,
        QString *errorText, bool acceptNonZeroExit = false)
    {
        QProcess process;
        process.start(program, arguments);
        if (!process.waitForStarted(3000)) {
            if (errorText) {
                *errorText = process.errorString();
            }
            return false;
        }
        if (!process.waitForFinished(5000)) {
            process.kill();
            process.waitForFinished(1000);
            if (errorText) {
                *errorText = i18n("Command timed out");
            }
            return false;
        }
        if (standardOutput) {
            *standardOutput = process.readAllStandardOutput();
        }
        const bool succeeded = process.exitStatus() == QProcess::NormalExit
            && (acceptNonZeroExit || process.exitCode() == 0);
        if (!succeeded && errorText) {
            const QString standardError = QString::fromUtf8(process.readAllStandardError()).trimmed();
            *errorText = standardError.isEmpty()
                ? i18n("Command exited with status %1", process.exitCode())
                : standardError;
        }
        return succeeded;
    }

    int m_away = 15, m_idle = 15, m_startupGrace = 10, m_present = 750, m_osdDelay = 1000, m_osdCooldown = 5, m_screenOffDelay = 750;
    bool m_enabled = true, m_dryRun = true, m_lockScreen = true, m_turnOff = false, m_wake = true, m_wakeManualLock = false, m_showOsd = true;
    QString m_presentText, m_awayText;
    bool m_screenOffSupported = false;
    QString m_diagnosticSummary;
    bool m_serviceReady = false;
    QString m_serviceSummary;
    QString m_operationError;
};

K_PLUGIN_CLASS_WITH_JSON(HpdSettings, "kcm_thinkpadhpd.json")
#include "hpdsettings.moc"
