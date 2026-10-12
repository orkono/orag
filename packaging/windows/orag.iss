; ORAG installer for Windows (Inno Setup 6), Task 47 / D-001.
; Per-user, no administrator rights: installs to %LOCALAPPDATA%\Programs\ORAG.
; Data and models live in %USERPROFILE%\.orag and are never touched here.
; Build (CI, release.yml): iscc /DAppVersion=<version> /DSourceDir=<staged files> /DOutputDir=<dir> orag.iss

#ifndef AppVersion
  #error AppVersion is required (/DAppVersion=x.y.z)
#endif
#ifndef SourceDir
  #error SourceDir is required (/DSourceDir=<folder with orag.exe and the docs>)
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif

[Setup]
; Fixed for every version: an upgrade replaces the previous install.
AppId={{CAF4F279-A0E3-4C66-8DB5-D4320AB22C64}
AppName=ORAG
AppVersion={#AppVersion}
AppVerName=ORAG {#AppVersion}
AppPublisher=orkono
AppPublisherURL=https://github.com/orkono/orag
DefaultDirName={localappdata}\Programs\ORAG
DefaultGroupName=ORAG
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19045
OutputDir={#OutputDir}
OutputBaseFilename=ORAG-Setup-{#AppVersion}
SetupIconFile={#SourceDir}\orag-icon.ico
UninstallDisplayIcon={app}\orag-icon.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; A running ORAG is closed before its files are replaced. Graceful Ctrl-Break
; is not possible from here; interrupted indexing resumes on the next start.
CloseApplications=yes
CloseApplicationsFilter=orag.exe
RestartApplications=no
LicenseFile={#SourceDir}\LICENSE-MIT
InfoAfterFile={#SourceDir}\README-Windows.txt

[Languages]
Name: "turkish"; MessagesFile: "compiler:Languages\Turkish.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
turkish.DesktopIcon=Masaüstüne kısayol oluştur
english.DesktopIcon=Create a desktop shortcut
turkish.FetchModels=ORAG modellerini indir (internet)
english.FetchModels=Download ORAG models (internet)
turkish.DataKept=ORAG kaldırıldı. Belgeleriniz ve modelleriniz %1 klasöründe duruyor; artık gerekmiyorsa o klasörü kendiniz silebilirsiniz.
english.DataKept=ORAG was removed. Your documents and models are still in %1; delete that folder yourself if you no longer need them.

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopIcon}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\orag.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\orag-icon.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README-Windows.txt"; DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\CHANGELOG.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD-PARTY-LICENSES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\docs\api.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#SourceDir}\fetch-model-pack.ps1"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; The console window shows the logs; closing it stops ORAG.
Name: "{group}\ORAG"; Filename: "{app}\orag.exe"; Parameters: "start"; WorkingDir: "{app}"; IconFilename: "{app}\orag-icon.ico"
; Downloading is an explicit user action (D-012): the script asks first.
Name: "{group}\{cm:FetchModels}"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -NoExit -File ""{app}\fetch-model-pack.ps1"""; WorkingDir: "{app}"; IconFilename: "{app}\orag-icon.ico"
Name: "{group}\{cm:UninstallProgram,ORAG}"; Filename: "{uninstallexe}"
Name: "{userdesktop}\ORAG"; Filename: "{app}\orag.exe"; Parameters: "start"; WorkingDir: "{app}"; IconFilename: "{app}\orag-icon.ico"; Tasks: desktopicon

[Code]
// Where ORAG keeps documents and models: ORAG_HOME if set, else %USERPROFILE%\.orag.
function DataFolder(): String;
begin
  Result := GetEnv('ORAG_HOME');
  if Result = '' then
    Result := ExpandConstant('{%USERPROFILE}') + '\.orag';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
    MsgBox(FmtMessage(CustomMessage('DataKept'), [DataFolder()]), mbInformation, MB_OK);
end;
