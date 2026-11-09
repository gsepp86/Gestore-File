; Script di Inno Setup per creare l'installer di "Gestore File CP"
; Prima di usarlo serve "Gestore File CP.exe" nella cartella dist\ (lo prepara compila.bat)

#define AppName "Gestore File"
#define AppFileName "Gestore File CP"
#define AppVersion "1.0"
#define AppPublisher "Peppe CP"
#define AppExe "Gestore File CP.exe"

[Setup]
AppId={{6F1B7C52-3A4D-4E8B-9C21-5D7A0E4B1F93}
AppName={#AppFileName}
AppVersion={#AppVersion}
AppVerName={#AppFileName} {#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\{#AppFileName}
DefaultGroupName={#AppFileName}
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename=Installa Gestore File CP {#AppVersion}
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppFileName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
VersionInfoVersion=1.0.0.0
VersionInfoProductName={#AppFileName}

[Languages]
Name: "italian"; MessagesFile: "compiler:Languages\Italian.isl"

[Tasks]
Name: "desktopicon"; Description: "Crea un'icona sul Desktop"; GroupDescription: "Collegamenti:"; Flags: unchecked

[Files]
Source: "..\dist\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppFileName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppFileName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Avvia {#AppFileName}"; Flags: nowait postinstall skipifsilent
