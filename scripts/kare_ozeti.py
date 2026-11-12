"""Kare kaydını özetler.

Eski kullanım (değişmedi):  python scripts/kare_ozeti.py kayıt.csv [başlangıç_sn] [hz]

Yeni seçenekler:
  --start SN    başlangıç saniyesi (varsayılan 4.0; konumsal 2. argümanla aynı)
  --hz HZ       yenileme hızı (varsayılan 180; konumsal 3. argümanla aynı)
  --skip SN     --start'tan sonraki ilk SN saniyeyi at (açılış kuyruğu için)
  --auto-tail   açılış kuyruğunu kendisi bul ve at (baştaki çok yavaş kareler)
  --json        makine okur çıktı (scripts/kare-ab.ps1 kullanır), tek satır JSON

Kayıt scripts/kare-olcumu.ps1 ile alınır. Varsayılan bütçe 180 Hz (5,56 ms).
"norm" sütunu = kare başına cpu_ms / build_ms: işlemci hız durumu (≈10 ms ↔ ≈20 ms)
değişse de turlar arası karşılaştırılabilsin diye hıza göre düzeltilmiş çizim süresi.
"""
import argparse, csv, json, math, statistics, sys


def pct(v, p):
    v = sorted(v)
    return v[min(len(v) - 1, int(round(p / 100 * (len(v) - 1))))] if v else float('nan')


def detect_tail(rows):
    """Baştaki açılış kuyruğunu bulur: sonraki yarının medyanının 2,5 katından
    yavaş kareler; art arda 5 'sakin' kare görülünce kuyruk bitmiş sayılır.
    Atılacak kare sayısını döndürür."""
    cpu = [float(r['cpu_ms']) for r in rows if r['cpu_ms']]
    if len(cpu) < 20:
        return 0
    ref = statistics.median(cpu[len(cpu) // 2:])
    limit = max(2.5 * ref, ref + 3.0)
    calm = 0
    for i, c in enumerate(cpu):
        calm = calm + 1 if c <= limit else 0
        if calm >= 5:
            return max(0, i - 4)
    return 0


def summarize(path, start=4.0, hz=180.0, skip=0.0, auto_tail=False):
    with open(path, newline='') as f:
        rows = [r for r in csv.DictReader(f) if r['t_s'] and float(r['t_s']) >= start]
    if skip > 0 and rows:
        t_cut = float(rows[0]['t_s']) + skip
        rows = [r for r in rows if float(r['t_s']) >= t_cut]
    dropped = 0
    if auto_tail:
        dropped = detect_tail(rows)
        rows = rows[dropped:]

    budget = 1000.0 / hz
    cpu = [float(r['cpu_ms']) for r in rows if r['cpu_ms']]
    iv = [float(r['interval_ms']) for r in rows if r['interval_ms']]
    build = [float(r['build_ms']) for r in rows if r.get('build_ms')]
    textms = [float(r['text_ms']) for r in rows if r.get('text_ms')]
    textn = [int(r['text_n']) for r in rows if r.get('text_n')]
    # Kare başına hıza göre düzeltilmiş çizim süresi (build_ms ≈ 0 olanlar hariç).
    norm = [float(r['cpu_ms']) / float(r['build_ms'])
            for r in rows if r['cpu_ms'] and r.get('build_ms') and float(r['build_ms']) > 0.05]

    dur = (float(rows[-1]['t_s']) - float(rows[0]['t_s'])) if len(rows) > 1 else 0
    moving = [i for i in iv if i <= 100]
    out = {
        'dosya': path, 'kare': len(rows), 'sure_s': round(dur, 3),
        'kare_sn': round(len(rows) / dur, 2) if dur else 0.0,
        'hz': hz, 'butce_ms': round(budget, 3), 'atilan_kuyruk': dropped,
        'asan': sum(1 for c in cpu if c > budget) if cpu else 0,
    }
    if cpu:
        out.update(cpu_p50=pct(cpu, 50), cpu_p95=pct(cpu, 95), cpu_p99=pct(cpu, 99), cpu_max=max(cpu))
    if build:
        out.update(build_p50=pct(build, 50), build_p95=pct(build, 95))
    if norm:
        out.update(norm_p50=pct(norm, 50), norm_p95=pct(norm, 95), norm_p99=pct(norm, 99))
    if textms:
        out.update(text_p50=pct(textms, 50), text_p95=pct(textms, 95), text_n_p50=pct(textn, 50) if textn else 0)
    if iv:
        out.update(aralik_p50=pct(moving, 50), aralik_p95=pct(moving, 95),
                   aralik_max=max(moving) if moving else 0.0, uzun_durak=len(iv) - len(moving))
    return out


def clean(o):
    """JSON için NaN -> None."""
    return {k: (None if isinstance(v, float) and math.isnan(v) else v) for k, v in o.items()}


def print_text(o):
    tail = f"; açılış kuyruğu {o['atilan_kuyruk']} kare atıldı" if o['atilan_kuyruk'] else ""
    print(f"kare: {o['kare']} ({o['sure_s']:.2f} sn içinde, ort. {o['kare_sn']:.1f}/sn); "
          f"bütçe {o['butce_ms']:.2f} ms @ {o['hz']:.0f} Hz{tail}")
    if 'cpu_p50' in o:
        print(f"çizim ms  p50 {o['cpu_p50']:.2f}  p95 {o['cpu_p95']:.2f}  p99 {o['cpu_p99']:.2f}  "
              f"en çok {o['cpu_max']:.2f}  bütçeyi aşan {o['asan']}/{o['kare']}")
    if 'build_p50' in o:
        print(f"kurma ms  p50 {o['build_p50']:.2f}  p95 {o['build_p95']:.2f}  (geri kalanı yerleşim + boyama)")
    if 'norm_p50' in o:
        print(f"norm      p50 {o['norm_p50']:.2f}  p95 {o['norm_p95']:.2f}  p99 {o['norm_p99']:.2f}  "
              f"(çizim ms / kurma ms; hız durumundan bağımsız)")
    if 'text_p50' in o:
        print(f"yazı ms   p50 {o['text_p50']:.2f}  p95 {o['text_p95']:.2f}  (karede ~{o['text_n_p50']} satır yazı)")
    if 'aralik_p50' in o:
        print(f"aralık ms p50 {o['aralik_p50']:.2f}  p95 {o['aralik_p95']:.2f}  en çok {o['aralik_max']:.2f}  "
              f"(100 ms'den uzun durak: {o['uzun_durak']})")


def main():
    ap = argparse.ArgumentParser(description="Openify kare kaydı özeti")
    ap.add_argument('path')
    ap.add_argument('pos_start', nargs='?', type=float)
    ap.add_argument('pos_hz', nargs='?', type=float)
    ap.add_argument('--start', type=float)
    ap.add_argument('--hz', type=float)
    ap.add_argument('--skip', type=float, default=0.0)
    ap.add_argument('--auto-tail', action='store_true')
    ap.add_argument('--json', action='store_true')
    a = ap.parse_args()
    start = a.start if a.start is not None else (a.pos_start if a.pos_start is not None else 4.0)
    hz = a.hz if a.hz is not None else (a.pos_hz if a.pos_hz is not None else 180.0)
    try:
        sys.stdout.reconfigure(encoding='utf-8')
    except Exception:
        pass
    o = summarize(a.path, start, hz, a.skip, a.auto_tail)
    if a.json:
        print(json.dumps(clean(o), ensure_ascii=False))
    else:
        print_text(o)


if __name__ == '__main__':
    main()
