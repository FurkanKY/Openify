//! Spirc yaşam döngüsü kararları (ağsız): kapalıyken Spirc KURULMAZ, açıkken
//! her oturuma bir tane, oturum yenilenince yenisi, eskimiş kurulum reddedilir.

use super::*;

fn host(enabled: bool) -> ConnectHost {
    let mut h = ConnectHost::new();
    h.enabled = enabled;
    h
}

#[test]
fn disabled_never_launches_whatever_the_session_looks_like() {
    let mut h = host(false);
    for (ready, id_ok) in [(false, false), (true, false), (false, true), (true, true)] {
        assert_eq!(h.decide(ready, id_ok), Decision::Nothing);
    }
    assert!(!h.is_running(), "kapalıyken hiç Spirc yok");
    // Oturum yenilense de (yeniden bağlanma) kapalı kalır.
    h.new_session();
    assert_eq!(h.decide(true, true), Decision::Nothing);
    assert!(!h.is_running());
}

#[test]
fn enabled_launches_exactly_once_when_the_session_is_ready_with_the_stable_id() {
    let mut h = host(true);
    assert_eq!(h.decide(false, false), Decision::Nothing, "oturum yok: bekle");
    let Decision::Launch(first) = h.decide(true, true) else { panic!("başlamalı") };
    assert!(h.is_running() && h.is_current(first));
    assert_eq!(h.decide(true, true), Decision::Nothing, "ikinci kez kurulmaz");
    assert!(h.started(first));
    assert_eq!(h.decide(true, true), Decision::Nothing, "çalışırken de kurulmaz");
}

#[test]
fn a_session_without_the_stable_id_is_restarted_once_not_in_a_loop() {
    let mut h = host(true);
    assert_eq!(h.decide(true, false), Decision::RestartSession);
    assert_eq!(h.decide(true, false), Decision::Nothing, "aynı oturumda ikinci kez istenmez");
    assert!(!h.is_running(), "kimlik uyuşmazken Spirc kurulmaz (hayalet aygıt olmasın)");
    // Yeni oturum kimlikle geldi: kurulur.
    h.new_session();
    assert!(matches!(h.decide(true, true), Decision::Launch(_)));
}

#[test]
fn a_replaced_session_gets_a_fresh_spirc_and_the_old_start_is_rejected() {
    let mut h = host(true);
    let Decision::Launch(old) = h.decide(true, true) else { panic!() };
    assert!(h.started(old));
    // Oturum düştü / yenilendi: eski Spirc bırakılır, yenisi kurulur.
    h.stop();
    assert!(!h.is_running() && !h.is_current(old));
    h.new_session();
    let Decision::Launch(new) = h.decide(true, true) else { panic!("yeniden kurulmalı") };
    assert!(new > old, "her Spirc'in kendi sıra numarası var");
    // Eski Spirc'in gecikmiş olayı / bitişi yenisini bozmaz.
    assert!(!h.is_current(old));
    assert!(!h.ended(old));
    assert!(h.is_current(new));
    assert!(h.started(new));
}

#[test]
fn a_start_that_finishes_after_the_stop_is_rejected() {
    let mut h = host(true);
    let Decision::Launch(g) = h.decide(true, true) else { panic!() };
    h.stop(); // kurulum sürerken kapatıldı / oturum değişti
    assert!(!h.started(g), "geç gelen Spirc kabul edilmez (çağıran kapatır)");
    assert!(!h.is_running());
}

#[test]
fn disabling_while_running_asks_to_stop() {
    let mut h = host(true);
    let Decision::Launch(g) = h.decide(true, true) else { panic!() };
    assert!(h.started(g));
    h.enabled = false;
    assert_eq!(h.decide(true, true), Decision::Stop);
    h.stop();
    assert_eq!(h.decide(true, true), Decision::Nothing);
}

#[test]
fn a_failed_or_ended_spirc_is_not_retried_by_itself() {
    let mut h = host(true);
    let Decision::Launch(g) = h.decide(true, true) else { panic!() };
    assert!(h.failed(g));
    assert!(!h.is_running());
    // Aynı oturumda yeniden kurulamaz (dealer tek kullanımlık): yeni oturum ister.
    assert_eq!(h.decide(true, true), Decision::RestartSession);
    h.new_session();
    let Decision::Launch(g2) = h.decide(true, true) else { panic!("yeni oturumda kurulur") };
    assert!(h.started(g2));
    assert!(h.ended(g2));
    assert!(!h.ended(g2), "ikinci bitiş sayılmaz");
}

#[test]
fn the_name_is_what_the_engine_was_told_and_defaults_to_the_app_name() {
    let h = ConnectHost::new();
    assert_eq!(h.name, crate::brand::APP_NAME);
    assert!(!h.enabled, "varsayılan KAPALI");
}

#[test]
fn the_session_id_is_only_set_while_connect_is_on() {
    // Kapalı: bugünkü rastgele kimlik (hiçbir şey değişmez).
    apply_connect_identity(false, "0a1b2c3d-0000-4000-8000-000000000000");
    assert_eq!(session_device_id(), None);
    apply_connect_identity(true, "");
    assert_eq!(session_device_id(), None, "kimlik yoksa kalıcı kimlik de yok");
    apply_connect_identity(true, "0a1b2c3d-0000-4000-8000-000000000000");
    assert_eq!(session_device_id().as_deref(), Some("0a1b2c3d-0000-4000-8000-000000000000"));
    apply_connect_identity(false, "0a1b2c3d-0000-4000-8000-000000000000");
    assert_eq!(session_device_id(), None);
}

#[test]
fn turning_connect_off_and_on_again_needs_a_fresh_session() {
    // Dealer kurucusu oturum başına tek kullanımlık: aynı oturumda ikinci Spirc
    // kurulamaz, yeniden açmak oturumu yeniler.
    let mut h = host(true);
    let Decision::Launch(g) = h.decide(true, true) else { panic!() };
    assert!(h.started(g));
    h.enabled = false;
    assert_eq!(h.decide(true, true), Decision::Stop);
    h.stop();
    h.enabled = true;
    assert_eq!(h.decide(true, true), Decision::RestartSession, "aynı oturumda ikinci Spirc yok");
    assert_eq!(h.decide(true, true), Decision::Nothing, "yeniden bağlanma bir kez istenir");
    h.new_session();
    assert!(matches!(h.decide(true, true), Decision::Launch(_)), "yeni oturumda kurulur");
}
