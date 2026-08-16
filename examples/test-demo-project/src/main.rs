use iddqd::BiHashItem;
use kiss3d::{egui, prelude::*};

use crate::{asset_handler::fetch_asset_bytes, dialogue::NPCData, time_stepper::FixedTimeStepper};
use fold::stream::Stream;

mod asset_handler;
mod dialogue;
mod time_stepper;
mod util;

#[kiss3d::main]
async fn main() {
    let mut window = Window::new("Bog-a-thon").await;
    let mut camera = PanZoomCamera2d::new(Vec2::ZERO, 5.0);
    let mut scene = SceneNode2d::empty();

    let mut texture_manager = TextureManager::new();
    let sprite_texture = texture_manager.add_image_from_memory_pixelated(
        &fetch_asset_bytes("my_sprite.png")
            .await
            .expect("Should exist"),
        "my_sprite",
    );
    let mut square = scene.add_rectangle(10.0, 10.0).set_texture(sprite_texture);
    let mut time_stepper = FixedTimeStepper::default();
    let test_npc = NPCData::from_json_slice(
        &fetch_asset_bytes("dialogue1.json")
            .await
            .expect("should exist"),
    )
    .expect("should parse");
    let font = Font::default();

    while window.render_2d(&mut scene, &mut camera).await {
        for event in window.events().iter() {
            match event.value {
                WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                    if let Some((x, y)) = window.cursor_pos() {
                        let screen_position = Vec2::new(x as f32, y as f32);
                        let world_position =
                            camera.unproject(screen_position, window.size().as_vec2());
                        log!("{:?}", world_position);
                    }
                }
                _ => {}
            }
        }

        while time_stepper.step() {
            square.rotate(0.1);
        }

        if let Some((label, dialogue_node)) = test_npc.get_current_dialog() {
            // Draw UI
            window.draw_ui(|ctx| {
                egui::Window::new("Kiss3d egui Example")
                    .default_width(300.0)
                    .show(ctx, |ui| {
                        // Rotation control
                        ui.label(label);

                        ui.separator();
                        ui.label(dialogue_node.key2().text.clone());
                    });
            });
        }
    }
}
