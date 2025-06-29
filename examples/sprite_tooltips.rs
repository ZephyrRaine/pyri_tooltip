//! A demonstration of tooltip support for both sprites and UI elements.

use bevy::prelude::*;
use bevy_sprite::Anchor;
use bevy_transform::components::GlobalTransform;
use bevy_window::PrimaryWindow;
use pyri_tooltip::prelude::*;

fn main() -> AppExit {
    App::new()
        .add_plugins((DefaultPlugins, TooltipPlugin::default()))
        .add_systems(Startup, setup)
        .add_systems(Update, debug_sprite_hover)
        .run()
}

fn setup(mut commands: Commands) {
    // Camera
    commands.spawn(Camera2d);

    // Spawn some sprites with tooltips
    commands.spawn((
        Sprite {
            color: Color::srgb(1.0, 0.0, 0.0),
            custom_size: Some(Vec2::new(50.0, 50.0)),
            ..default()
        },
        Transform::from_translation(Vec3::new(-100.0, 0.0, 0.0)),
        Tooltip::cursor("Red Sprite - This tooltip should work!"),
        DebugSprite {
            original_color: Color::srgb(1.0, 0.0, 0.0),
        },
    ));

    commands.spawn((
        Sprite {
            color: Color::srgb(0.0, 1.0, 0.0),
            custom_size: Some(Vec2::new(80.0, 80.0)),
            ..default()
        },
        Transform::from_translation(Vec3::new(100.0, 0.0, 0.0)),
        Tooltip::fixed(Anchor::TopCenter, "Green Sprite - Fixed tooltip!"),
        DebugSprite {
            original_color: Color::srgb(0.0, 1.0, 0.0),
        },
    ));

    // Also add a UI tooltip for comparison
    commands.spawn((
        Node {
            width: Val::Px(100.0),
            height: Val::Px(50.0),
            position_type: PositionType::Absolute,
            top: Val::Px(50.0),
            left: Val::Px(50.0),
            border: UiRect::all(Val::Px(2.0)),
            ..default()
        },
        BackgroundColor(Color::srgb(0.0, 0.0, 1.0)),
        BorderColor(Color::srgb(1.0, 1.0, 1.0)),
        Interaction::default(),
        Tooltip::cursor("UI Element - Should still work!"),
    ));
}

#[derive(Component)]
struct DebugSprite {
    original_color: Color,
}

fn debug_sprite_hover(
    mut sprite_query: Query<(Entity, &mut Sprite, &DebugSprite, &GlobalTransform), With<Tooltip>>,
    camera_query: Query<(&Camera, &Transform)>,
    window_query: Query<&Window>,
    primary_window_query: Query<Entity, With<PrimaryWindow>>,
) {
    // Get cursor position
    let cursor_pos = window_query
        .iter()
        .find_map(|window| window.cursor_position())
        .unwrap_or_default();

    // Find camera
    let camera = camera_query.iter().next();

    if let Some((camera_component, camera_transform)) = camera {
        // Get the window for coordinate conversion
        let window = match camera_component.target {
            bevy_render::camera::RenderTarget::Window(bevy_window::WindowRef::Primary) => {
                if let Ok(window_entity) = primary_window_query.single() {
                    window_query.get(window_entity).ok()
                } else {
                    None
                }
            }
            bevy_render::camera::RenderTarget::Window(bevy_window::WindowRef::Entity(
                window_entity,
            )) => window_query.get(window_entity).ok(),
            _ => None,
        };

        if let Some(window) = window {
            // Check each sprite with tooltip
            for (_entity, mut sprite, debug, transform) in &mut sprite_query {
                // Reset to original color first
                sprite.color = debug.original_color;

                // Manual world to screen conversion (same as tooltip system)
                let world_pos = transform.translation();
                let center = camera_transform.translation.truncate();
                let half_width = (window.width() / 2.0) * camera_transform.scale.x;
                let half_height = (window.height() / 2.0) * camera_transform.scale.y;
                let left = center.x - half_width;
                let bottom = center.y - half_height;

                let screen_pos = Vec2::new(
                    (world_pos.x - left) / camera_transform.scale.x,
                    window.height() - ((world_pos.y - bottom) / camera_transform.scale.y),
                );

                let sprite_size = sprite.custom_size.unwrap_or(Vec2::new(32.0, 32.0));
                let half_size = sprite_size * 0.5;
                let min = screen_pos - half_size;
                let max = screen_pos + half_size;

                if cursor_pos.x >= min.x
                    && cursor_pos.x <= max.x
                    && cursor_pos.y >= min.y
                    && cursor_pos.y <= max.y
                {
                    // Change color to indicate hover
                    sprite.color = Color::srgb(1.0, 1.0, 0.0); // Yellow when hovered
                }
            }
        }
    }
}
