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

public:
    HpdSettings(QObject *parent, const KPluginMetaData &data)
        : KQuickConfigModule(parent, data)
    {
        KLocalizedString::setApplicationDomain("kcm_thinkpadhpd");
        setButtons(Apply | Default | Help);
    }

    void load() override
    {
        QProcess process;
        process.start(QStringLiteral("thinkpad-hpd"), {QStringLiteral("settings"), QStringLiteral("get")});
        process.waitForFinished();
        const auto object = QJsonDocument::fromJson(process.readAllStandardOutput()).object();
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
        QProcess diagnostics;
        diagnostics.start(QStringLiteral("thinkpad-hpd"), {QStringLiteral("diagnose")});
        diagnostics.waitForFinished();
        const auto diagnosticObject = QJsonDocument::fromJson(diagnostics.readAllStandardOutput()).object();
        m_screenOffSupported = diagnosticObject[QStringLiteral("screen_off_supported")].toBool(false);
        const auto reason = diagnosticObject[QStringLiteral("screen_off_block_reason")].toString();
        const auto sensorAvailable = diagnosticObject[QStringLiteral("sensor")].toObject()[QStringLiteral("available")].toBool(false);
        m_diagnosticSummary = sensorAvailable ? i18n("Presence sensor detected") : i18n("Presence sensor unavailable");
        if (!reason.isEmpty()) {
            m_diagnosticSummary += QStringLiteral(". ")
                + i18n("Automatic display power-off is blocked on AMDGPU Wayland after observed display failures.");
        }
        QProcess daemonState;
        daemonState.start(QStringLiteral("systemctl"), {QStringLiteral("is-active"), QStringLiteral("thinkpad-hpd.service")});
        daemonState.waitForFinished();
        QProcess daemonEnabled;
        daemonEnabled.start(QStringLiteral("systemctl"), {QStringLiteral("is-enabled"), QStringLiteral("thinkpad-hpd.service")});
        daemonEnabled.waitForFinished();
        const bool daemonActive = daemonState.exitCode() == 0;
        const bool daemonMasked = QString::fromUtf8(daemonEnabled.readAllStandardOutput()).trimmed() == QStringLiteral("masked");
        QProcess agentState;
        agentState.start(QStringLiteral("systemctl"), {QStringLiteral("--user"), QStringLiteral("is-active"), QStringLiteral("thinkpad-hpd-agent.service")});
        agentState.waitForFinished();
        const bool agentActive = agentState.exitCode() == 0;
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
        QProcess process;
        process.start(QStringLiteral("thinkpad-hpd"), {QStringLiteral("settings"), QStringLiteral("set"),
            QStringLiteral("--json"), QString::fromUtf8(QJsonDocument(object).toJson(QJsonDocument::Compact))});
        process.waitForFinished();
        if (process.exitCode() == 0) {
            const QStringList serviceArguments = m_enabled
                ? QStringList{QStringLiteral("--user"), QStringLiteral("enable"), QStringLiteral("--now"), QStringLiteral("thinkpad-hpd-agent.service")}
                : QStringList{QStringLiteral("--user"), QStringLiteral("disable"), QStringLiteral("--now"), QStringLiteral("thinkpad-hpd-agent.service")};
            const int serviceResult = QProcess::execute(QStringLiteral("systemctl"), serviceArguments);
            if (serviceResult == 0) {
                setNeedsSave(false);
                load();
            } else {
                qWarning() << "Failed to update HPD desktop agent state, exit code:" << serviceResult;
                setNeedsSave(true);
            }
        } else {
            qWarning() << "Failed to save HPD settings:" << process.readAllStandardError();
            setNeedsSave(true);
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
    int m_away = 15, m_idle = 15, m_startupGrace = 10, m_present = 750, m_osdDelay = 1000, m_osdCooldown = 5, m_screenOffDelay = 750;
    bool m_enabled = true, m_dryRun = true, m_lockScreen = true, m_turnOff = false, m_wake = true, m_wakeManualLock = false, m_showOsd = true;
    QString m_presentText, m_awayText;
    bool m_screenOffSupported = false;
    QString m_diagnosticSummary;
    bool m_serviceReady = false;
    QString m_serviceSummary;
};

K_PLUGIN_CLASS_WITH_JSON(HpdSettings, "kcm_thinkpadhpd.json")
#include "hpdsettings.moc"
