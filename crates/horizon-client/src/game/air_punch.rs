// air_punch (Android yamasi, tam animasyon). Orijinal kod.
// Yerde ve havada ayri vurus + idle sallanma + easing + yer/hava gecisi.
// Kullanim: game.rs'te `to_world * self.air_punch_offset(hand, left, attack) * vanilla_pose`.
use super::*;
use glam::{Mat4, Vec3};
use horizon_client::first_person::Hand;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

// ============================ AYARLAR ============================
// Oyunda yon ters cikarsa SADECE bunlari degistir (1.0 <-> -1.0).
// Hangisi ne yapiyor:
//   SIGN_X     elin yana kaymasi (vurusta ice/disa)
//   SIGN_Y     elin yukari/asagi kaymasi
//   SIGN_Z     elin one/geri itilmesi (vurusta one gitmeli)
//   SIGN_PITCH el bilegi yukari/asagi donusu (vurusta asagi inmeli)
//   SIGN_YAW   elin ice/disa donusu
//   SIGN_ROLL  el yan yatisi
const SIGN_X: f32 = 1.0;
const SIGN_Y: f32 = 1.0;
const SIGN_Z: f32 = 1.0;
const SIGN_PITCH: f32 = 1.0;
const SIGN_YAW: f32 = 1.0;
const SIGN_ROLL: f32 = 1.0;
// Dönüş merkezi (kol/el konumu, vanilla uzayinda, sag el icin). Tahmini.
// El "savruluyor"/yanlis noktadan donuyorsa bunu degistir.
const PIVOT: [f32; 3] = [0.64, -0.6, -0.72];
// Genel siddet carpani (1.0 = oldugu gibi, 0.5 = yarisi).
const SCALE: f32 = 1.0;
// =================================================================

// ==== ANIM BEGIN (glam/Game'e bagimli degil, ayri test edilir) ====
// (zaman sn, [x, y, z, pitch, yaw, roll], easing)
// easing: 0 dogrusal, 1 quad-out, 2 quad-in-out, 3 cubic-out, 4 back-out
type Key = (f32, [f32; 6], u8);

const GROUND_IDLE: [Key; 3] = [
    (0.0, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0),
    (1.0, [0.0, 0.01, 0.0, 0.6, 0.0, 0.3], 2),
    (2.0, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 2),
];
const AIR_IDLE: [Key; 3] = [
    (0.0, [0.0, 0.04, 0.0, -8.0, 0.0, 3.0], 0),
    (0.6, [0.0, 0.06, -0.01, -12.0, 2.0, -3.0], 2),
    (1.2, [0.0, 0.04, 0.0, -8.0, 0.0, 3.0], 2),
];
const GROUND_SWING: [Key; 4] = [
    (0.00, [0.00, 0.00, 0.00, 0.0, 0.0, 0.0], 0),
    (0.08, [0.02, 0.05, 0.03, -16.0, 6.0, 4.0], 1),
    (0.16, [-0.08, -0.07, -0.10, 52.0, -18.0, -10.0], 4),
    (0.38, [0.00, 0.00, 0.00, 0.0, 0.0, 0.0], 2),
];
const AIR_SWING: [Key; 5] = [
    (0.00, [0.00, 0.04, 0.00, -8.0, 0.0, 3.0], 0),
    (0.10, [0.04, 0.10, 0.05, -28.0, 12.0, 8.0], 1),
    (0.20, [-0.12, -0.10, -0.14, 68.0, -26.0, -16.0], 4),
    (0.30, [-0.16, -0.12, -0.12, 82.0, -36.0, -22.0], 3),
    (0.50, [0.00, 0.04, 0.00, -8.0, 0.0, 3.0], 2),
];

fn ease(kind: u8, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match kind {
        1 => 1.0 - (1.0 - t) * (1.0 - t),
        2 => {
            if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
            }
        }
        3 => 1.0 - (1.0 - t).powi(3),
        4 => {
            let c1 = 1.70158_f32;
            let c3 = c1 + 1.0;
            1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
        }
        _ => t,
    }
}

fn mix(a: [f32; 6], b: [f32; 6], k: f32) -> [f32; 6] {
    let mut out = [0.0_f32; 6];
    for i in 0..6 {
        out[i] = a[i] + (b[i] - a[i]) * k;
    }
    out
}

fn duration(keys: &[Key]) -> f32 {
    keys[keys.len() - 1].0
}

fn sample(keys: &[Key], time: f32, looping: bool) -> [f32; 6] {
    let dur = duration(keys);
    let mut t = time.max(0.0);
    if looping && dur > 0.0 {
        t %= dur;
    } else if t >= dur {
        return keys[keys.len() - 1].1;
    }
    for w in keys.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if t >= a.0 && t < b.0 {
            let k = ease(b.2, (t - a.0) / (b.0 - a.0));
            return mix(a.1, b.1, k);
        }
    }
    keys[keys.len() - 1].1
}

/// clock: saniye (idle dongusu icin), swing: 0..1 vanilla vurus ilerlemesi,
/// air: 0 = yerde, 1 = havada (yumusatilmis). Cikis: [x, y, z, pitch, yaw, roll].
fn pose(clock: f32, swing: f32, air: f32) -> [f32; 6] {
    let ground_idle = sample(&GROUND_IDLE, clock, true);
    let air_idle = sample(&AIR_IDLE, clock, true);
    let (g, a) = if swing > 0.0 {
        // Vurusun basinda ve sonunda bosta pozuna yumusak gecis.
        let ramp = (swing / 0.12).min((1.0 - swing) / 0.12).clamp(0.0, 1.0);
        let gs = sample(&GROUND_SWING, swing * duration(&GROUND_SWING), false);
        let as_ = sample(&AIR_SWING, swing * duration(&AIR_SWING), false);
        (mix(ground_idle, gs, ramp), mix(air_idle, as_, ramp))
    } else {
        (ground_idle, air_idle)
    };
    mix(g, a, air.clamp(0.0, 1.0))
}
// ==== ANIM END ====

struct State {
    last: Instant,
    air: f32,
}
static STATE: Mutex<Option<State>> = Mutex::new(None);
static START: OnceLock<Instant> = OnceLock::new();

impl Game<'_> {
    pub(super) fn air_punch_offset(&self, hand: Hand, left: bool, attack: f32) -> Mat4 {
        // Yalniz ana el. Zaman durumu da yalniz burada ilerler.
        if hand != Hand::Main {
            return Mat4::IDENTITY;
        }
        let Some(pl) = self.player.as_ref() else {
            return Mat4::IDENTITY;
        };
        let target = if pl.ground.on_ground { 0.0 } else { 1.0 };
        let now = Instant::now();
        let clock = now.duration_since(*START.get_or_init(|| now)).as_secs_f32();
        let air = {
            let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
            let st = g.get_or_insert_with(|| State { last: now, air: 0.0 });
            // arm ve item ayni karede iki kez cagirir; ikincide dt ~ 0, sorun yok.
            let dt = now.duration_since(st.last).as_secs_f32().min(0.1);
            st.last = now;
            st.air += (target - st.air) * (1.0 - (-dt * 12.0).exp());
            st.air
        };
        let mut p = pose(clock, attack.clamp(0.0, 1.0), air);
        for v in p.iter_mut() {
            *v *= SCALE;
        }
        let f = if left { -1.0 } else { 1.0 }; // sol elde yatay eksenleri aynala
        let pv = Vec3::new(PIVOT[0] * f, PIVOT[1], PIVOT[2]);
        let rot = Mat4::from_rotation_y((f * SIGN_YAW * p[4]).to_radians())
            * Mat4::from_rotation_x((SIGN_PITCH * p[3]).to_radians())
            * Mat4::from_rotation_z((f * SIGN_ROLL * p[5]).to_radians());
        Mat4::from_translation(Vec3::new(f * SIGN_X * p[0], SIGN_Y * p[1], SIGN_Z * p[2]))
            * Mat4::from_translation(pv)
            * rot
            * Mat4::from_translation(-pv)
    }
}
