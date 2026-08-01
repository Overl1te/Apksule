; Apksule Windows installer
; Builds a single-file setup with .apk file association and context menu.

#define MyAppName "Apksule"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "Overl1te"
#define MyAppURL "https://github.com/Overl1te/Apksule"
#define MyAppExeName "apksule.exe"

#ifndef SourceDir
  #define SourceDir "..\target\release"
#endif

#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

#ifndef IconFile
  #define IconFile "..\assets\apksule.ico"
#endif

[Setup]
AppId={{A8C4E2B1-5D7F-4A9C-8E31-2B6F0D9A4C71}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
LicenseFile=..\LICENSE-MIT
OutputDir={#OutputDir}
OutputBaseFilename=Apksule-Setup-{#MyAppVersion}
SetupIconFile={#IconFile}
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesAssociations=yes
MinVersion=10.0
VersionInfoVersion={#MyAppVersion}.0
VersionInfoCompany={#MyAppPublisher}
VersionInfoDescription={#MyAppName} installer
VersionInfoProductName={#MyAppName}
VersionInfoProductVersion={#MyAppVersion}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "fileassoc"; Description: "Associate .apk files with Apksule (open on double-click)"; GroupDescription: "Integration:"; Flags: checkedonce
Name: "contextmenu"; Description: "Add ""Open with Apksule"" to the .apk context menu"; GroupDescription: "Integration:"; Flags: checkedonce

[Files]
Source: "{#SourceDir}\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\apksule.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\apksule.ico"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\apksule.ico"; Tasks: desktopicon

[Registry]
; ProgID for Apksule
Root: HKCR; Subkey: "Apksule.APK"; ValueType: string; ValueName: ""; ValueData: "Android Package (Apksule)"; Flags: uninsdeletekey; Tasks: fileassoc contextmenu
Root: HKCR; Subkey: "Apksule.APK\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\apksule.ico,0"; Tasks: fileassoc contextmenu
Root: HKCR; Subkey: "Apksule.APK\shell\open"; ValueType: string; ValueName: ""; ValueData: "Open with Apksule"; Tasks: fileassoc
Root: HKCR; Subkey: "Apksule.APK\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: fileassoc

; Make .apk open with Apksule by default
Root: HKCR; Subkey: ".apk"; ValueType: string; ValueName: ""; ValueData: "Apksule.APK"; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKCR; Subkey: ".apk\OpenWithProgids"; ValueType: string; ValueName: "Apksule.APK"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc contextmenu

; Explicit context-menu verb for .apk (works even if another ProgID owns the extension)
Root: HKCR; Subkey: "SystemFileAssociations\.apk\shell\Apksule"; ValueType: string; ValueName: ""; ValueData: "Open with Apksule"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKCR; Subkey: "SystemFileAssociations\.apk\shell\Apksule"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\apksule.ico"; Tasks: contextmenu
Root: HKCR; Subkey: "SystemFileAssociations\.apk\shell\Apksule\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: contextmenu

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
