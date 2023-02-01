use async_std::channel::{bounded, Receiver, Sender};
use async_std::stream::StreamExt;
use async_std::sync::RwLock;
use async_std::task;
use evdev::Device;
use futures_util;
use std::env;
use std::time::Duration;
use zbus::{dbus_proxy, Connection};

const LIGHT_UP: i32 = 100;
const LIGHT_DOWN: i32 = 0;
const LIGHT_TIMEOUT: u64 = 4000;
const LIGHT_STEPS: i32 = 50;

#[dbus_proxy(
    interface = "org.freedesktop.UPower.KbdBacklight",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/KbdBacklight"
)]
trait KbdBacklight {
    /// SetBrightness method
    fn set_brightness(&self, value: i32) -> zbus::Result<()>;

    /// BrightnessChanged signal
    #[dbus_proxy(signal)]
    fn brightness_changed(&self, value: i32) -> zbus::Result<()>;
}

async fn dbus_job(rx: Receiver<i32>) -> Result<(), zbus::Error> {
    let conn = Connection::system().await?;
    let proxy = KbdBacklightProxy::new(&conn).await?;
    let mut brightness_changed = proxy.receive_brightness_changed().await?;
    let brlock = RwLock::new(-1);

    futures_util::try_join!(
        async {
            while let Some(signal) = brightness_changed.next().await {
                let mut b = brlock.write().await;
                let args = signal.args()?;

                *b = args.value;
            }
            Ok::<(), zbus::Error>(())
        },
        async {
            while let Ok(i) = rx.recv().await {
                let b = brlock.read().await;
                if *b != i {
                    let _ = proxy.set_brightness(i).await?;
                }
            }
            Ok::<(), zbus::Error>(())
        },
    )?;

    Ok(())
}

async fn device_loop(device: String, tx: Sender<i32>) {
    let mut device = Device::open(device).expect("couldnt open");
    let mut guard: Option<task::JoinHandle<_>> = None;
    while let Ok(_fe) = device.fetch_events() {
        if let Some(h) = guard {
            h.cancel().await;
        }
        tx.send(LIGHT_UP).await.expect("couldn't send");
        let ttx = tx.clone();
        guard = Some(task::spawn(async move {
            task::sleep(Duration::from_millis(LIGHT_TIMEOUT)).await;
            for i in 0..LIGHT_STEPS {
                let light: i32 = (LIGHT_STEPS - i - 1) * (LIGHT_UP - LIGHT_DOWN) / LIGHT_STEPS;
                let _ = ttx.send(light).await;
                task::sleep(Duration::from_millis(50)).await;
            }
        }));
    }
}

#[async_std::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let (tx, rx): (Sender<i32>, Receiver<i32>) = bounded(1);
    task::spawn(dbus_job(rx));
    task::spawn(device_loop(args[1].clone(), tx)).await;
}
