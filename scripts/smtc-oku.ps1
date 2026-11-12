# Windows medya kontrolünü dışarıdan okur (Windows PowerShell 5.1: WinRT
# yalnız orada doğrudan kullanılabiliyor). sistem-sinama.ps1 çağırır.
#   -Mode oku    oturumları ve bizimkinin bilgisini yazar (ad, sanatçı, albüm,
#                kapak baytı, durum, konum / süre)
#   -Mode dugme  bizim oturumda "oynat / duraklat" düğmesine basar; her basışın
#                UTC anını yazar (uygulamanın kaydıyla karşılaştırmak için)
# Yalnız kimliği -Kimlik olan oturuma dokunur (kullanıcının çalan müziğine değil).
param(
    [string]$Mode = "oku",
    [string]$Kimlik = "Openify.Test",
    [int]$Tekrar = 5
)
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
    $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
})[0]
function Await($op, [Type]$type) {
    $task = $asTaskGeneric.MakeGenericMethod($type).Invoke($null, @($op))
    $task.Wait(-1) | Out-Null
    $task.Result
}
[Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager, Windows.Media.Control, ContentType = WindowsRuntime] | Out-Null
[Windows.Storage.Streams.IRandomAccessStreamWithContentType, Windows.Storage.Streams, ContentType = WindowsRuntime] | Out-Null
$mgr = Await ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager]::RequestAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager])

function Ours {
    foreach ($s in $mgr.GetSessions()) { if ($s.SourceAppUserModelId -eq $Kimlik) { return $s } }
    return $null
}

if ($Mode -eq "oku") {
    $ids = @($mgr.GetSessions() | ForEach-Object { $_.SourceAppUserModelId })
    "  oturumlar: $($ids -join ', ')"
    $s = Ours
    if (-not $s) { "  BİZİM OTURUM YOK ($Kimlik)"; exit 1 }
    $p = Await ($s.TryGetMediaPropertiesAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties])
    $thumb = "yok"
    if ($null -ne $p.Thumbnail) {
        try {
            $stream = Await ($p.Thumbnail.OpenReadAsync()) ([Windows.Storage.Streams.IRandomAccessStreamWithContentType])
            # WinRT akışını .NET akışına çevirip baytları say (PowerShell 5.1
            # arayüzün Size özelliğini göstermiyor).
            # Yansıma ile çağrı: CLR, COM nesnesini arayüze kendisi çeviriyor.
            $m = [System.IO.WindowsRuntimeStreamExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsStreamForRead' -and $_.GetParameters().Count -eq 1 }
            $net = $m.Invoke($null, @($stream))
            $mem = New-Object System.IO.MemoryStream
            $net.CopyTo($mem)
            $bytes = $mem.ToArray()
            $magic = if ($bytes.Length -ge 2 -and $bytes[0] -eq 0xFF -and $bytes[1] -eq 0xD8) { "JPEG" } elseif ($bytes.Length -ge 4 -and $bytes[1] -eq 0x50) { "PNG" } else { "?" }
            $thumb = "$($bytes.Length) bayt ($magic)"
        } catch { $thumb = "var ama okunamadı: $($_.Exception.InnerException.Message)" }
    }
    $info = $s.GetPlaybackInfo()
    $tl = $s.GetTimelineProperties()
    "  ad: $($p.Title) | sanatçı: $($p.Artist) | albüm: $($p.AlbumTitle) | tür: $($p.PlaybackType)"
    "  kapak: $thumb"
    "  durum: $($info.PlaybackStatus) | düğmeler: oynat $($info.Controls.IsPlayEnabled), duraklat $($info.Controls.IsPauseEnabled), sonraki $($info.Controls.IsNextEnabled), önceki $($info.Controls.IsPreviousEnabled), konum $($info.Controls.IsPlaybackPositionEnabled)"
    "  konum: $($tl.Position) / $($tl.EndTime) (güncellendi $($tl.LastUpdatedTime.ToUniversalTime().ToString('HH:mm:ss.fff')) UTC, şimdi $([DateTime]::UtcNow.ToString('HH:mm:ss.fff')))"
}
elseif ($Mode -eq "bekle") {
    # Uygulama açılmadan başlatılır: bizim oturumda ad ve kapak ilk görüldüğü
    # anları yazar (uygulamanın kaydındaki "medya kontrolü bilgisi" anıyla
    # karşılaştırmak için).
    $deadline = [DateTime]::UtcNow.AddSeconds(40)
    $titleAt = $null; $thumbAt = $null
    while ([DateTime]::UtcNow -lt $deadline -and -not $thumbAt) {
        $s = Ours
        if ($s) {
            $p = Await ($s.TryGetMediaPropertiesAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties])
            $now = [DateTime]::UtcNow
            if (-not $titleAt -and $p.Title) { $titleAt = $now; "ad göründü $($now.ToString('HH:mm:ss.fff')): $($p.Title) / $($p.Artist)" }
            if ($titleAt -and $null -ne $p.Thumbnail) { $thumbAt = $now; "kapak göründü $($now.ToString('HH:mm:ss.fff'))" }
        }
        Start-Sleep -Milliseconds 5
    }
    if (-not $titleAt) { "ad hiç görünmedi" }
}
elseif ($Mode -eq "sar") {
    # Medya kontrolünden 60. saniyeye sar, sonra konum çizelgesini oku.
    $s = Ours
    if (-not $s) { "BİZİM OTURUM YOK ($Kimlik)"; exit 1 }
    $at = [DateTime]::UtcNow
    $ok = Await ($s.TryChangePlaybackPositionAsync([long]600000000)) ([bool])
    Start-Sleep -Milliseconds 400
    $tl = $s.GetTimelineProperties()
    $age = ([DateTime]::UtcNow - $tl.LastUpdatedTime.UtcDateTime).TotalSeconds
    "sar $($at.ToString('HH:mm:ss.fff')) kabul: $ok; çizelge: $($tl.Position) (güncellendi $($tl.LastUpdatedTime.ToUniversalTime().ToString('HH:mm:ss.fff')), şimdi tahmini $([math]::Round($tl.Position.TotalSeconds + $age, 2)) sn)"
}
elseif ($Mode -eq "dugme") {
    $s = Ours
    if (-not $s) { "BİZİM OTURUM YOK ($Kimlik)"; exit 1 }
    for ($n = 0; $n -lt $Tekrar; $n++) {
        $at = [DateTime]::UtcNow
        $ok = Await ($s.TryTogglePlayPauseAsync()) ([bool])
        "bas $($at.ToString('HH:mm:ss.fff')) kabul: $ok ($([math]::Round(([DateTime]::UtcNow - $at).TotalMilliseconds, 1)) ms)"
        Start-Sleep -Milliseconds 250
    }
}
