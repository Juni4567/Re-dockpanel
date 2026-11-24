// DockPanel Desktop - WSL2 & Service Prerequisites Pascal Script
// Included in Inno Setup offline installer wizard

function IsWSLDistroRegistered(DistroName: String): Boolean;
var
  ResultCode: Integer;
begin
  // Check if dockpanel-desktop distro exists by executing a test echo command inside it
  Result := Exec(ExpandConstant('{sys}\wsl.exe'), '-d ' + DistroName + ' -- echo ok', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) and (ResultCode = 0);
end;

procedure CheckAndEnableWSL;
var
  ResultCode: Integer;
begin
  Log('Checking WSL status on host system...');
  if not Exec(ExpandConstant('{sys}\wsl.exe'), '--status', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) or (ResultCode <> 0) then
  begin
    Log('WSL status check failed or WSL not enabled. Invoking wsl.exe --install --no-distribution...');
    if Exec(ExpandConstant('{sys}\wsl.exe'), '--install --no-distribution', '', SW_SHOWNORMAL, ewWaitUntilTerminated, ResultCode) then
    begin
      Log(Format('wsl.exe --install finished with exit code: %d', [ResultCode]));
      if ResultCode = 0 then
      begin
        MsgBox('WSL2 has been enabled. A system reboot may be required to complete setup.', mbInformation, MB_OK);
      end;
    end
    else
    begin
      Log('Failed to execute wsl.exe --install');
    end;
  end
  else
  begin
    Log('WSL is already active and healthy.');
  end;
end;

procedure ImportWSLDistro;
var
  DistroDir: String;
  ArchiveFile: String;
  ResultCode: Integer;
  Params: String;
begin
  DistroDir := ExpandConstant('{app}\runtime\distro');
  ArchiveFile := ExpandConstant('{app}\runtime\dockpanel-runtime.tar.gz');

  if IsWSLDistroRegistered('dockpanel-desktop') then
  begin
    Log('WSL distro "dockpanel-desktop" is already registered. Skipping import.');
    Exit;
  end;

  if not FileExists(ArchiveFile) then
  begin
    Log('WSL runtime archive not found: ' + ArchiveFile);
    Exit;
  end;

  if not DirExists(DistroDir) then
  begin
    ForceDirectories(DistroDir);
  end;

  Log('Importing WSL2 distro dockpanel-desktop from runtime archive...');
  Params := Format('--import dockpanel-desktop "%s" "%s" --version 2', [DistroDir, ArchiveFile]);
  if Exec(ExpandConstant('{sys}\wsl.exe'), Params, '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
  begin
    Log(Format('wsl.exe --import finished with exit code: %d', [ResultCode]));
  end
  else
  begin
    Log('Failed to execute wsl.exe --import');
  end;
end;

procedure InstallHostHelperService;
var
  HelperExe: String;
  ResultCode: Integer;
  CreateParams: String;
begin
  HelperExe := ExpandConstant('{app}\dockpanel-host-helper.exe');
  if not FileExists(HelperExe) then
  begin
    Log('Host helper executable not found: ' + HelperExe);
    Exit;
  end;

  Log('Creating Windows Service DockPanelHostHelper...');
  CreateParams := Format('create DockPanelHostHelper binPath= "\"%s\"" start= auto obj= "LocalSystem"', [HelperExe]);
  Exec(ExpandConstant('{sys}\sc.exe'), CreateParams, '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Log(Format('sc.exe create finished with exit code: %d', [ResultCode]));

  Log('Starting Windows Service DockPanelHostHelper...');
  Exec(ExpandConstant('{sys}\sc.exe'), 'start DockPanelHostHelper', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Log(Format('sc.exe start finished with exit code: %d', [ResultCode]));
end;

procedure InstallRootCA;
var
  MkcertExe: String;
  ResultCode: Integer;
begin
  MkcertExe := ExpandConstant('{app}\mkcert.exe');
  if not FileExists(MkcertExe) then
  begin
    Log('mkcert executable not found: ' + MkcertExe);
    Exit;
  end;

  Log('Registering development CA in Windows root store using mkcert -install...');
  if Exec(MkcertExe, '-install', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
  begin
    Log(Format('mkcert -install finished with exit code: %d', [ResultCode]));
  end
  else
  begin
    Log('Failed to execute mkcert -install');
  end;
end;

procedure PerformUninstallCleanup;
var
  ResultCode: Integer;
begin
  Log('Stopping and removing DockPanelHostHelper service...');
  Exec(ExpandConstant('{sys}\sc.exe'), 'stop DockPanelHostHelper', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\sc.exe'), 'delete DockPanelHostHelper', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);

  Log('Unregistering WSL distro dockpanel-desktop...');
  Exec(ExpandConstant('{sys}\wsl.exe'), '--unregister dockpanel-desktop', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;
