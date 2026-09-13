#ifndef AppVersion
  #define AppVersion Trim(FileRead(FileOpen("version.txt")))
#endif
#ifndef VersionNumeric
  #define VersionNumeric AppVersion
#endif
#ifndef AppName
  #define AppName "BlubberBound"
#endif
#ifndef PackageId
  #define PackageId "{{596690BB-D4AC-4C45-A35A-5C39B158156C}"
#endif
#ifndef Executable
  #define Executable "BlubberBound.exe"
#endif
#ifndef PayloadDirectory
  #define PayloadDirectory "BlubberBound"
#endif
#ifndef PayloadRoot
  #define PayloadRoot "dist\" + PayloadDirectory
#endif
#ifndef InstallerName
  #define InstallerName "BlubberBound-Setup"
#endif
#ifndef StorageId
  #define StorageId "BlubberBound"
#endif
#ifndef Publisher
  #define Publisher "mkiera"
#endif
#ifndef RepositoryUrl
  #define RepositoryUrl "https://github.com/mkiera/BlubberBound"
#endif

[Setup]
AppId={#PackageId}
AppName={#AppName}
AppVersion={#AppVersion}
VersionInfoVersion={#VersionNumeric}
AppPublisher={#Publisher}
AppPublisherURL={#RepositoryUrl}
DefaultDirName={localappdata}\Programs\{#StorageId}
DefaultGroupName={#AppName}
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\app\{#Executable}
OutputDir=dist_installer
OutputBaseFilename={#InstallerName}
SetupIconFile=icon.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
DisableProgramGroupPage=yes
CloseApplications=yes
RestartApplications=no
UsePreviousAppDir=yes
UsePreviousTasks=yes

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#PayloadRoot}\*"; DestDir: "{app}\app"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\app\{#Executable}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\app\{#Executable}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\{#Executable}"; ValueType: string; ValueData: "{app}\app\{#Executable}"; Flags: uninsdeletekey

[Run]
Filename: "{app}\app\{#Executable}"; Flags: nowait runasoriginaluser

[Code]
var
  SavedPayload: String;
  LivePayload: String;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Backup: String;
begin
  Result := '';
  LivePayload := ExpandConstant('{app}\app');
  Backup := ExpandConstant('{app}\app.old');
  if DirExists(Backup) then begin
    if not DirExists(LivePayload) then begin
      if not RenameFile(Backup, LivePayload) then
        Result := 'The previous application could not be restored. Close the application and try again.';
    end else
      Result := 'An earlier installation left app.old beside the application. Keep a copy of that folder and remove it before retrying.';
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Backup: String;
begin
  if CurStep = ssInstall then begin
    Backup := ExpandConstant('{app}\app.old');
    if DirExists(LivePayload) then begin
      if not RenameFile(LivePayload, Backup) then
        RaiseException('Could not move the current application. Close the application and try again.');
      SavedPayload := Backup;
    end;
  end else if CurStep = ssPostInstall then begin
    if SavedPayload <> '' then begin
      DelTree(SavedPayload, True, True, True);
      SavedPayload := '';
    end;
  end;
end;

procedure DeinitializeSetup();
begin
  if SavedPayload <> '' then begin
    DelTree(LivePayload, True, True, True);
    if not RenameFile(SavedPayload, LivePayload) then
      Log('The previous payload remains in app.old and must be restored before retrying.');
  end;
end;
