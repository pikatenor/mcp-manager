use tray_icon::menu::{Menu, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub fn install() {
    match create() {
        Ok(icon) => std::mem::forget(icon),
        Err(error) => eprintln!("tray icon failed: {error}"),
    }
}

fn create() -> Result<TrayIcon, Box<dyn std::error::Error>> {
    let icon = load_icon()?;
    let open = MenuItem::with_id("open", "Open", true, None);
    let copy = MenuItem::with_id("copy-endpoint", "Copy endpoint", true, None);
    let quit = MenuItem::with_id("quit", "Quit", true, None);
    let menu = Menu::new();
    menu.append(&open)?;
    menu.append(&copy)?;
    menu.append(&quit)?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("MCP Manager")
        .with_icon(icon)
        .with_menu_on_left_click(true)
        .build()?;
    std::mem::forget(open);
    std::mem::forget(copy);
    std::mem::forget(quit);
    Ok(tray)
}

fn load_icon() -> Result<Icon, Box<dyn std::error::Error>> {
    let bytes = include_bytes!("../icons/32x32.png");
    let image = image::load_from_memory(bytes)?.into_rgba8();
    let (width, height) = image.dimensions();
    Ok(Icon::from_rgba(image.into_raw(), width, height)?)
}
