use kiss3d::{egui, prelude::*};

use crate::{
    asset_handler::fetch_asset_bytes, dialogue::NPCData, history::DialogueHistory,
    sprites::preload_sprites, time_stepper::FixedTimeStepper,
};

mod asset_handler;
mod dialogue;
mod history;
mod sprite_list;
mod sprites;
mod time_stepper;
mod util;

#[kiss3d::main]
async fn main() {
    let mut window = Window::new("Bog-a-thon").await;
    let mut camera = PanZoomCamera2d::new(Vec2::ZERO, 5.0);
    let mut scene = SceneNode2d::empty();

    let mut texture_manager = TextureManager::new();
    preload_sprites("sprites.json", &mut texture_manager).await;
    let sprite_texture = texture_manager.get("my_sprite").expect("should exist");
    let mut square = scene
        .add_rectangle(256.0 / 2., 288.0 / 2.)
        .set_texture(sprite_texture);
    let mut time_stepper = FixedTimeStepper::default();
    let test_npc = NPCData::from_json_slice(
        &fetch_asset_bytes("dialogue1.json")
            .await
            .expect("should exist"),
    )
    .expect("should parse");

    // stores all dialog history in the visual novel
    let mut history = DialogueHistory::open(std::env::temp_dir().join("bogkit-dialogue.db"));
    history.enter(&test_npc);

    // What the portrait is showing, so the texture is only swapped when the
    // derived answer actually changes rather than every frame.
    let mut shown_sprite: Option<String> = None;

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

        // Draw UI
        window.draw_ui(|ctx| {
            egui::Window::new("Kiss3d egui Example")
                .default_width(300.0)
                .show(ctx, |ui| {
                    let (step, label) = history.enter(&test_npc);

                    // What the player picked this frame, applied after the
                    // borrow of the node ends.
                    let mut chosen: Option<String> = None;
                    let mut go_back = false;
                    let mut restart = false;

                    if let Some(node) = test_npc.node(&label) {
                        ui.label(node.label());

                        ui.separator();
                        ui.label(node.data().text.clone());
                        for next_label in &node.data().next {
                            if ui.button(next_label).clicked() {
                                chosen = Some(next_label.clone());
                            }
                        }
                    }

                    ui.separator();
                    ui.horizontal(|ui| {
                        // step 0 is the opening line: nothing behind it
                        go_back = ui
                            .add_enabled(step > 0, egui::Button::new("< back"))
                            .clicked();
                        restart = ui
                            .add_enabled(step > 0, egui::Button::new("restart"))
                            .clicked();
                    });
                    ui.label(format!(
                        "path: {}",
                        history.path(test_npc.name()).join(" > ")
                    ));

                    ui.separator();
                    ui.label("nodes on a live path (retracted by rewind):");
                    for (visited_label, count) in history.visited_counts() {
                        ui.label(format!("  {visited_label}: {count}"));
                    }

                    if let Some(next) = chosen {
                        let _ = history.advance(&test_npc, &next);
                    } else if go_back {
                        history.rewind(test_npc.name());
                    } else if restart {
                        history.restart(test_npc.name());
                    }
                });
        });

        // The portrait is derived, never assigned: whatever the sprite branch
        // holds after this frame's clicks is what goes up, rewinds included.
        let wanted = history.current_sprite(test_npc.name());
        if wanted != shown_sprite {
            if let Some(name) = &wanted
                && let Some(texture) = texture_manager.get(name)
            {
                square.set_texture(texture);
            }
            shown_sprite = wanted;
        }
    }
}
