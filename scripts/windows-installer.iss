#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef PackageDir
  #error PackageDir must point to the packaged application folder
#endif
[Setup]
AppId={{473FBA01-4930-49CC-A844-F14EFDD3A9D6}
AppName=FreeCut
AppVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\FreeCut
DefaultGroupName=FreeCut
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19045
OutputDir={#PackageDir}\..
OutputBaseFilename=FreeCut-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=no
LicenseFile={#PackageDir}\Licenses\FreeCut-GPL-3.0.txt
UninstallDisplayIcon={app}\freecut.exe
[Files]
Source: "{#PackageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
[Tasks]
Name: desktopicon; Description: "Create a desktop shortcut"; Flags: unchecked
[Icons]
Name: "{group}\FreeCut"; Filename: "{app}\freecut.exe"
Name: "{group}\FreeCut (CPU)"; Filename: "{app}\freecut.exe"; Parameters: "--software --low-memory"
Name: "{autodesktop}\FreeCut"; Filename: "{app}\freecut.exe"; Tasks: desktopicon
