use bevy_app::{App, PreUpdate};
#[cfg(feature = "bevy_reflect")]
use bevy_ecs::reflect::ReflectResource;
use bevy_ecs::{
    entity::Entity,
    event::{Event, EventReader, EventWriter},
    query::With,
    resource::Resource,
    schedule::{IntoScheduleConfigs as _, common_conditions::on_event},
    system::{Query, Res, ResMut},
};
use bevy_math::Vec2;
use bevy_render::{
    camera::{Camera, RenderTarget},
    view::Visibility,
};
use bevy_time::Time;
use bevy_ui::{Interaction, UiStack};
use bevy_window::{PrimaryWindow, Window, WindowRef};
// Add these imports for sprite support:
use crate::{Tooltip, TooltipContent, TooltipSettings, TooltipSystems, rich_text::RichText};
use bevy_sprite::Sprite;
use bevy_transform::components::{GlobalTransform, Transform};
use tiny_bail::prelude::*;

pub(super) fn plugin(app: &mut App) {
    #[cfg(feature = "bevy_reflect")]
    app.register_type::<TooltipContext>();
    app.init_resource::<TooltipContext>();
    app.add_event::<HideTooltip>();
    app.add_event::<ShowTooltip>();
    app.add_systems(
        PreUpdate,
        (
            update_tooltip_context,
            hide_tooltip.run_if(on_event::<HideTooltip>),
            show_tooltip.run_if(on_event::<ShowTooltip>),
        )
            .chain()
            .in_set(TooltipSystems::Content),
    );
}

/// A [`Resource`] that contains the current values in use by the tooltip system.
#[derive(Resource, Clone, Debug)]
#[cfg_attr(
    feature = "bevy_reflect",
    derive(bevy_reflect::Reflect),
    reflect(Resource)
)]
pub(crate) struct TooltipContext {
    /// The current state of the tooltip system.
    pub(crate) state: TooltipState,
    /// The current or previous target entity being interacted with.
    pub(crate) target: Entity,
    /// The remaining duration of the current activation delay or transfer timeout (in milliseconds).
    timer: u16,
    /// The current cursor position or activation point.
    pub(crate) cursor_pos: Vec2,
    /// The current tooltip parameters.
    pub(crate) tooltip: Tooltip,
}

impl Default for TooltipContext {
    fn default() -> Self {
        Self {
            state: TooltipState::Inactive,
            target: Entity::PLACEHOLDER,
            timer: 0,
            cursor_pos: Vec2::ZERO,
            tooltip: Tooltip::cursor(Entity::PLACEHOLDER),
        }
    }
}

// Helper function to get window from camera target
fn get_window_from_camera<'a>(
    camera: &Camera,
    primary_window_query: &Query<Entity, With<PrimaryWindow>>,
    window_query: &'a Query<&Window>,
) -> Option<&'a Window> {
    match camera.target {
        RenderTarget::Window(WindowRef::Primary) => {
            let window_entity = primary_window_query.single().ok()?;
            window_query.get(window_entity).ok()
        }
        RenderTarget::Window(WindowRef::Entity(window_entity)) => {
            window_query.get(window_entity).ok()
        }
        _ => None,
    }
}

// Helper function to get current cursor position from camera
fn get_current_cursor_pos(
    camera: &Camera,
    primary_window_query: &Query<Entity, With<PrimaryWindow>>,
    window_query: &Query<&Window>,
) -> Vec2 {
    match camera.target {
        RenderTarget::Window(WindowRef::Primary) => {
            if let Ok(window_entity) = primary_window_query.single() {
                if let Ok(window) = window_query.get(window_entity) {
                    return window.cursor_position().unwrap_or_default();
                }
            }
        }
        RenderTarget::Window(WindowRef::Entity(window_entity)) => {
            if let Ok(window) = window_query.get(window_entity) {
                return window.cursor_position().unwrap_or_default();
            }
        }
        _ => {}
    }
    Vec2::default()
}

// Helper function to determine tooltip state transition
fn should_activate_immediately(
    tooltip: &Tooltip,
    ctx: &TooltipContext,
    target_entity: Entity,
) -> bool {
    tooltip.activation.delay == 0
        || (matches!(ctx.state, TooltipState::Inactive)
            && ctx.timer > 0
            && ctx.tooltip.transfer.layer >= tooltip.transfer.layer
            && (matches!((ctx.tooltip.transfer.group, tooltip.transfer.group), (Some(x), Some(y)) if x == y)
                || ctx.target == target_entity))
}

// Helper function to apply tooltip transition
fn apply_tooltip_transition(
    ctx: &mut TooltipContext,
    entity: Entity,
    tooltip: &Tooltip,
    activate_immediately: bool,
) {
    ctx.state = if activate_immediately {
        TooltipState::Active
    } else {
        TooltipState::Delayed
    };
    ctx.target = entity;
    ctx.timer = tooltip.activation.delay;
    ctx.tooltip = tooltip.clone();
    ctx.tooltip.dismissal.on_distance *= ctx.tooltip.dismissal.on_distance;
}

fn sprite_contains_point(
    sprite: &Sprite,
    sprite_transform: &GlobalTransform,
    point: Vec2,
    window: &Window,
    camera_transform: &Transform,
) -> bool {
    // Manual world to screen conversion
    let world_pos = sprite_transform.translation();
    let center = camera_transform.translation.truncate();
    let half_width = (window.width() / 2.0) * camera_transform.scale.x;
    let half_height = (window.height() / 2.0) * camera_transform.scale.y;
    let left = center.x - half_width;
    let bottom = center.y - half_height;

    let screen_pos = Vec2::new(
        (world_pos.x - left) / camera_transform.scale.x,
        window.height() - ((world_pos.y - bottom) / camera_transform.scale.y),
    );

    // Calculate sprite size (use a reasonable default if not specified)
    let sprite_size = sprite.custom_size.unwrap_or(Vec2::new(32.0, 32.0));
    let half_size = sprite_size * 0.5;

    // For now, let's ignore the anchor and treat all sprites as center-anchored
    // This will help us debug if the basic coordinate conversion works
    let sprite_center = screen_pos;

    // Calculate bounds around the center
    let min = sprite_center - half_size;
    let max = sprite_center + half_size;

    point.x >= min.x && point.x <= max.x && point.y >= min.y && point.y <= max.y
}

fn update_tooltip_context(
    mut ctx: ResMut<TooltipContext>,
    mut hide_tooltip: EventWriter<HideTooltip>,
    mut show_tooltip: EventWriter<ShowTooltip>,
    primary: Res<TooltipSettings>,
    time: Res<Time>,
    ui_stack: Res<UiStack>,
    primary_window_query: Query<Entity, With<PrimaryWindow>>,
    window_query: Query<&Window>,
    camera_query: Query<(&Camera, &Transform)>,
    interaction_query: Query<(&Tooltip, &Interaction)>,
    sprite_query: Query<(Entity, &Tooltip, &Sprite, &GlobalTransform)>,
) {
    let old_active = matches!(ctx.state, TooltipState::Active);
    let old_target = ctx.target;
    let old_entity = match ctx.tooltip.content {
        TooltipContent::Primary(_) => primary.container,
        TooltipContent::Custom(id) => id,
    };

    // TODO: Reconsider whether this is the right way to detect cursor movement.
    // Detect cursor movement.
    for camera in &camera_query {
        let RenderTarget::Window(window) = camera.0.target else {
            continue;
        };
        let window = match window {
            WindowRef::Primary => cq!(primary_window_query.single()),
            WindowRef::Entity(id) => id,
        };
        let window = c!(window_query.get(window));
        cq!(window.focused);
        let cursor_pos = cq!(window.cursor_position());

        // Reset activation delay on cursor move.
        if ctx.cursor_pos != cursor_pos
            && matches!(ctx.state, TooltipState::Delayed)
            && ctx.tooltip.activation.reset_delay_on_cursor_move
        {
            ctx.timer = ctx.tooltip.activation.delay;
        }

        // Dismiss tooltip if cursor has left the activation radius.
        if matches!(ctx.state, TooltipState::Active)
            && ctx.cursor_pos.distance_squared(cursor_pos) > ctx.tooltip.dismissal.on_distance
        {
            ctx.state = TooltipState::Dismissed;
        }

        // Update cursor position.
        if !matches!(ctx.state, TooltipState::Active) {
            ctx.cursor_pos = cursor_pos;
        }

        break;
    }

    // Tick timer for transfer timeout / activation delay.
    if matches!(ctx.state, TooltipState::Inactive | TooltipState::Delayed) {
        ctx.timer = ctx.timer.saturating_sub(time.delta().as_millis() as u16);
        if matches!(ctx.state, TooltipState::Delayed) && ctx.timer == 0 {
            ctx.state = TooltipState::Active;
        }
    }

    // Find the highest entity in the `UiStack` that has a tooltip and is being interacted with.
    let mut found_target = false;
    for &entity in ui_stack.uinodes.iter().rev() {
        let (tooltip, interaction) = cq!(interaction_query.get(entity));
        match interaction {
            Interaction::Pressed if tooltip.dismissal.on_click => {
                ctx.target = entity;
                ctx.state = TooltipState::Dismissed;
                ctx.tooltip.transfer = tooltip.transfer;
                found_target = true;
                break;
            }
            Interaction::None => continue,
            _ => (),
        };

        // Still hovering the same target entity.
        if ctx.target == entity && !matches!(ctx.state, TooltipState::Inactive) {
            ctx.tooltip = tooltip.clone();
            ctx.tooltip.dismissal.on_distance *= ctx.tooltip.dismissal.on_distance;
            found_target = true;
            break;
        }

        // Switch to the new target entity.
        let activate_immediately = should_activate_immediately(tooltip, &ctx, entity);
        apply_tooltip_transition(&mut ctx, entity, tooltip, activate_immediately);
        found_target = true;
        break;
    }

    // If no UI tooltip found, check sprites
    if !found_target {
        // Find camera for coordinate conversion - try to find any suitable camera
        let camera = camera_query
            .iter()
            .find(|camera| {
                // Try to find a camera that targets the primary window or any window
                matches!(camera.0.target, RenderTarget::Window(_))
            })
            .or_else(|| {
                // Fallback: use the first camera available
                camera_query.iter().next()
            });

        if let Some(camera) = camera {
            // Get the current cursor position for sprite detection
            let current_cursor_pos =
                get_current_cursor_pos(camera.0, &primary_window_query, &window_query);

            // Check all sprites with tooltips for hover
            for (entity, tooltip, sprite, transform) in sprite_query.iter() {
                // Get the window for coordinate conversion
                let Some(window) =
                    get_window_from_camera(camera.0, &primary_window_query, &window_query)
                else {
                    continue;
                };

                // Skip sprites that aren't hovered (equivalent to `Interaction::None => continue`)
                // Use current cursor position, not cached one!
                if !sprite_contains_point(sprite, transform, current_cursor_pos, window, camera.1) {
                    continue;
                }

                // Still hovering the same target entity.
                if ctx.target == entity && !matches!(ctx.state, TooltipState::Inactive) {
                    ctx.tooltip = tooltip.clone();
                    ctx.tooltip.dismissal.on_distance *= ctx.tooltip.dismissal.on_distance;
                    found_target = true;
                    break;
                }

                // Switch to the new target entity.
                let activate_immediately = should_activate_immediately(tooltip, &ctx, entity);
                apply_tooltip_transition(&mut ctx, entity, tooltip, activate_immediately);
                found_target = true;
                break;
            }
        }
    }

    // There is no longer a target entity.
    if !found_target && !matches!(ctx.state, TooltipState::Inactive) {
        ctx.timer =
            if matches!(ctx.state, TooltipState::Active) || !ctx.tooltip.transfer.from_active {
                ctx.tooltip.transfer.timeout
            } else {
                0
            };
        ctx.state = TooltipState::Inactive;
    }

    // Update tooltip if it has a target, or was activated, dismissed, or changed targets.
    let new_active = matches!(ctx.state, TooltipState::Active);
    if old_active != new_active || old_target != ctx.target || found_target {
        if old_active {
            hide_tooltip.write(HideTooltip { entity: old_entity });
        }
        if new_active {
            show_tooltip.write(ShowTooltip);
        }
    }
}

/// The current state of the tooltip system.
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "bevy_reflect", derive(bevy_reflect::Reflect))]
pub(crate) enum TooltipState {
    /// There is no target entity being interacted with, and no active tooltip.
    Inactive,
    /// A target entity is being hovered, but its tooltip is not active yet.
    Delayed,
    /// A target entity is being hovered, and its tooltip is active.
    Active,
    /// A target entity is being interacted with, but its tooltip has been dismissed.
    Dismissed,
}

/// A buffered event sent when a tooltip should be hidden.
#[derive(Event)]
#[cfg_attr(feature = "bevy_reflect", derive(bevy_reflect::Reflect))]
struct HideTooltip {
    entity: Entity,
}

fn hide_tooltip(
    mut hide_tooltip: EventReader<HideTooltip>,
    mut visibility_query: Query<&mut Visibility>,
) {
    for event in hide_tooltip.read() {
        *cq!(visibility_query.get_mut(event.entity)) = Visibility::Hidden;
    }
}

/// A buffered event sent when a tooltip should be shown.
#[derive(Event)]
#[cfg_attr(feature = "bevy_reflect", derive(bevy_reflect::Reflect))]
struct ShowTooltip;

fn show_tooltip(
    mut ctx: ResMut<TooltipContext>,
    primary: Res<TooltipSettings>,
    mut text_query: Query<&mut RichText>,
    mut visibility_query: Query<&mut Visibility>,
) {
    let entity = match ctx.tooltip.content {
        TooltipContent::Primary(ref mut text) => {
            if let Ok(mut primary_text) = text_query.get_mut(primary.text) {
                *primary_text = core::mem::take(text);
            }
            primary.container
        }
        TooltipContent::Custom(id) => id,
    };
    *r!(visibility_query.get_mut(entity)) = Visibility::Visible;
}
