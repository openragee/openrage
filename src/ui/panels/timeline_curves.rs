use super::ParticleKfpSlot;

pub(super) fn behaviour_kpd_curves(
    b_idx: usize,
    b: &or_particle::behaviour::Behaviour,
) -> Vec<(
    String,
    ParticleKfpSlot,
    &Option<or_particle::keyframe::KeyframePropData>,
)> {
    use or_particle::behaviour::Behaviour;
    let prefix = format!("[{b_idx}] ");
    match b {
        Behaviour::Acceleration(d) => vec![
            (
                prefix.clone() + "Accel Min",
                ParticleKfpSlot::AccelMin,
                &d.xyz_min_keyframes,
            ),
            (
                prefix + "Accel Max",
                ParticleKfpSlot::AccelMax,
                &d.xyz_max_keyframes,
            ),
        ],
        Behaviour::Colour(d) => vec![
            (
                prefix.clone() + "RGBA Min",
                ParticleKfpSlot::ColourRgbaMin,
                &d.rgba_min_keyframes,
            ),
            (
                prefix.clone() + "RGBA Max",
                ParticleKfpSlot::ColourRgbaMax,
                &d.rgba_max_keyframes,
            ),
            (
                prefix + "Emissive",
                ParticleKfpSlot::ColourEmissive,
                &d.emissive_intensity_keyframes,
            ),
        ],
        Behaviour::Size(d) => vec![
            (
                prefix.clone() + "WHD Min",
                ParticleKfpSlot::SizeWhdMin,
                &d.whd_min_keyframes,
            ),
            (
                prefix.clone() + "WHD Max",
                ParticleKfpSlot::SizeWhdMax,
                &d.whd_max_keyframes,
            ),
            (
                prefix.clone() + "TBLR Scalar",
                ParticleKfpSlot::SizeTblrScalar,
                &d.tblr_scalar_keyframes,
            ),
            (
                prefix + "TBLR VelScalar",
                ParticleKfpSlot::SizeTblrVelScalar,
                &d.tblr_vel_scalar_keyframes,
            ),
        ],
        Behaviour::Rotation(d) => vec![
            (
                prefix.clone() + "Init Angle Min",
                ParticleKfpSlot::RotInitAngleMin,
                &d.initial_angle_min_keyframes,
            ),
            (
                prefix.clone() + "Init Angle Max",
                ParticleKfpSlot::RotInitAngleMax,
                &d.initial_angle_max_keyframes,
            ),
            (
                prefix.clone() + "Angle Min",
                ParticleKfpSlot::RotAngleMin,
                &d.angle_min_keyframes,
            ),
            (
                prefix + "Angle Max",
                ParticleKfpSlot::RotAngleMax,
                &d.angle_max_keyframes,
            ),
        ],
        Behaviour::Noise(d) => vec![
            (
                prefix.clone() + "Pos Noise Min",
                ParticleKfpSlot::NoisePosMin,
                &d.pos_noise_min_keyframes,
            ),
            (
                prefix.clone() + "Pos Noise Max",
                ParticleKfpSlot::NoisePosMax,
                &d.pos_noise_max_keyframes,
            ),
            (
                prefix.clone() + "Vel Noise Min",
                ParticleKfpSlot::NoiseVelMin,
                &d.vel_noise_min_keyframes,
            ),
            (
                prefix + "Vel Noise Max",
                ParticleKfpSlot::NoiseVelMax,
                &d.vel_noise_max_keyframes,
            ),
        ],
        Behaviour::Dampening(d) => vec![
            (
                prefix.clone() + "Damp Min",
                ParticleKfpSlot::DampMin,
                &d.xyz_min_keyframes,
            ),
            (
                prefix + "Damp Max",
                ParticleKfpSlot::DampMax,
                &d.xyz_max_keyframes,
            ),
        ],
        Behaviour::Wind(d) => vec![(
            prefix + "Influence",
            ParticleKfpSlot::WindInfluence,
            &d.influence_keyframes,
        )],
        Behaviour::Attractor(d) => vec![(
            prefix + "Strength",
            ParticleKfpSlot::AttractorStrength,
            &d.strength_keyframes,
        )],
        Behaviour::AnimateTexture(d) => vec![(
            prefix + "Anim Rate",
            ParticleKfpSlot::AnimateTextureRate,
            &d.anim_rate_keyframes,
        )],
        Behaviour::Collision(d) => vec![
            (
                prefix.clone() + "Bounciness",
                ParticleKfpSlot::CollisionBounciness,
                &d.bounciness_keyframes,
            ),
            (
                prefix + "Bounce Dir Var",
                ParticleKfpSlot::CollisionBounceDirVar,
                &d.bounce_dir_var_keyframes,
            ),
        ],
        Behaviour::MatrixWeight(d) => vec![(
            prefix + "Weight",
            ParticleKfpSlot::MatrixWeight,
            &d.weight_keyframes,
        )],
        Behaviour::ZCulling(d) => vec![
            (
                prefix.clone() + "Height",
                ParticleKfpSlot::ZCullingHeight,
                &d.height_keyframes,
            ),
            (
                prefix + "Fade Dist",
                ParticleKfpSlot::ZCullingFadeDist,
                &d.fade_dist_keyframes,
            ),
        ],
        Behaviour::Decal(d) => vec![
            (
                prefix.clone() + "Dimensions",
                ParticleKfpSlot::DecalDimensions,
                &d.dimensions_keyframes,
            ),
            (
                prefix + "Alpha",
                ParticleKfpSlot::DecalAlpha,
                &d.alpha_keyframes,
            ),
        ],
        _ => Vec::new(),
    }
}
