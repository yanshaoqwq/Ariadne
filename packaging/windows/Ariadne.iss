#ifndef AppVersion
  #error AppVersion must be provided
#endif
#ifndef AppName
  #error AppName must be provided from display_name.json
#endif
#ifndef SourceDir
  #error SourceDir must be provided
#endif
#ifndef OutputDir
  #error OutputDir must be provided
#endif

[Setup]
AppId={{5B02A5B5-58D7-4C53-AD60-4E47678EA56B}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Ariadne contributors
AppPublisherURL=https://github.com/yanshaoqwq/Ariadne
AppSupportURL=https://github.com/yanshaoqwq/Ariadne/issues
DefaultDirName={autopf}\Ariadne
DefaultGroupName=Ariadne
LicenseFile={#SourceDir}\LICENSE
OutputDir={#OutputDir}
OutputBaseFilename=Ariadne-{#AppVersion}-win-x64-setup
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
SetupIconFile={#SourceDir}\Integration\icons\ariadne.ico
UninstallDisplayIcon={app}\Ariadne.Desktop.exe
WizardStyle=modern
DisableProgramGroupPage=yes
#ifdef SignedBuild
SignTool=ariadnesign
SignedUninstaller=yes
#endif

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Ariadne"; Filename: "{app}\Ariadne.Desktop.exe"
Name: "{autodesktop}\Ariadne"; Filename: "{app}\Ariadne.Desktop.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Run]
Filename: "{app}\Ariadne.Desktop.exe"; Description: "{cm:LaunchProgram,Ariadne}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; 只清理安装目录；AppData 与用户项目不属于安装器所有权。
Type: filesandordirs; Name: "{app}"
