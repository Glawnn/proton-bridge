$ErrorActionPreference = "Stop"

Write-Host "=== E2E Proton Bridge ==="

$mockPort = 8999
$workDir = "$env:TEMP\proton-bridge-e2e"
$logFile = "$workDir\client-logs.txt"
$configDir = "$workDir\proton-bridge"
$configFile = "$configDir\config.toml"
$receivedPortFile = "$workDir\received_port.txt"
$binary = Join-Path (Get-Location) "target\release\proton-bridge.exe"

# --- Nettoyage ---
Remove-Item -Recurse -Force $workDir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $workDir | Out-Null
New-Item -ItemType Directory -Path $configDir | Out-Null

# --- Faux logs Proton avec un mapping actif sur le port 63537 ---
$logContent = @"
2026-11-17T19:53:58.706Z | INFO | PROCESS.COMM | Received PortForwarding Status 'SleepingUntilRefresh' triggered at '17/11/2026 19:53:58', Port pair 63537->63537, expiring in 00:01:00 at 17/11/2026 19:54:58 | {}
"@
Set-Content -Path $logFile -Value $logContent -Encoding UTF8

# --- Config TOML pointant vers le mock qBittorrent ---
# Les backslashes Windows sont remplacés par des slashes pour le TOML.
$logFilePosix = $logFile.Replace('\', '/')
$configContent = @"
[qbittorrent]
url = "http://localhost:$mockPort"
username = "admin"
password = "test"

[proton]
log_path = "$logFilePosix"

[general]
log_level = "debug"
poll_interval_seconds = 1
"@
Set-Content -Path $configFile -Value $configContent -Encoding UTF8

# Surcharger APPDATA pour que le binaire lise ce config temporaire
$env:APPDATA = $workDir

# --- Mock qBittorrent (HttpListener) ---
$listener = New-Object System.Net.HttpListener
$listener.Prefixes.Add("http://+:$mockPort/")
$listener.Start()

$listenerTask = [System.Threading.Tasks.Task]::Run({
    while ($listener.IsListening) {
        try {
            $context = $listener.GetContext()
            $request = $context.Request
            $response = $context.Response
            $path = $request.Url.AbsolutePath

            if ($path -eq "/api/v2/auth/login" -and $request.HttpMethod -eq "POST") {
                $response.StatusCode = 204
                $response.Close()
            }
            elseif ($path -eq "/api/v2/app/preferences" -and $request.HttpMethod -eq "GET") {
                $body = '{"listen_port": 6881}'
                $buffer = [System.Text.Encoding]::UTF8.GetBytes($body)
                $response.ContentLength64 = $buffer.Length
                $response.OutputStream.Write($buffer, 0, $buffer.Length)
                $response.Close()
            }
            elseif ($path -eq "/api/v2/app/setPreferences" -and $request.HttpMethod -eq "POST") {
                $reader = New-Object System.IO.StreamReader($request.InputStream)
                $body = $reader.ReadToEnd()
                if ($body -match 'listen_port":\s*(\d+)') {
                    Set-Content -Path $receivedPortFile -Value $matches[1] -Encoding UTF8
                }
                $response.StatusCode = 200
                $response.Close()
            }
            else {
                $response.StatusCode = 404
                $response.Close()
            }
        } catch {
            # Listener arrêté, on sort
            break
        }
    }
})

# --- Lancer proton-bridge ---
if (-not (Test-Path $binary)) {
    Write-Error "Binaire introuvable : $binary"
    exit 1
}

Write-Host "Lancement de $binary"
$process = Start-Process -FilePath $binary -WorkingDirectory (Get-Location) -PassThru

# --- Attendre jusqu'à 30s que le mock reçoive le nouveau port ---
$maxWait = 30
$elapsed = 0
while ($elapsed -lt $maxWait) {
    if (Test-Path $receivedPortFile) {
        break
    }
    Start-Sleep -Milliseconds 500
    $elapsed += 0.5
}

# --- Arrêter le binaire et le mock ---
if (-not $process.HasExited) {
    $process.Kill()
    $process.WaitForExit()
}
$listener.Stop()

# --- Vérifications ---
if (-not (Test-Path $receivedPortFile)) {
    Write-Error "E2E ECHEC : aucun appel setPreferences reçu du binaire"
    exit 1
}

$received = Get-Content $receivedPortFile -Raw | Trim
$expected = "63537"

if ($received -ne $expected) {
    Write-Error "E2E ECHEC : port attendu $expected, reçu $received"
    exit 1
}

Write-Host "E2E OK : qBittorrent port mis à jour à $received"
