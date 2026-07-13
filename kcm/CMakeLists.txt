cmake_minimum_required(VERSION 3.24)
project(kcm_thinkpadhpd VERSION 0.1.0)

find_package(ECM 6.0 REQUIRED NO_MODULE)
set(CMAKE_MODULE_PATH ${ECM_MODULE_PATH})
include(KDEInstallDirs6)
include(KDECMakeSettings)
include(KDECompilerSettings NO_POLICY_SCOPE)
find_package(Qt6 REQUIRED COMPONENTS Core Qml)
find_package(KF6 REQUIRED COMPONENTS CoreAddons KCMUtils I18n)

add_definitions(-DTRANSLATION_DOMAIN=\"kcm_thinkpadhpd\")

kcmutils_add_qml_kcm(kcm_thinkpadhpd SOURCES hpdsettings.cpp)
target_link_libraries(kcm_thinkpadhpd PRIVATE Qt6::Core Qt6::Qml KF6::CoreAddons KF6::KCMUtilsQuick KF6::I18n)
ki18n_install(po)
