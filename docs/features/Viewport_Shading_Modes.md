# Viewport shading modes

What each of the viewport's five shading modes draws, how a frame is made, and
what the viewport deliberately does not do.

The pills are `draw_shading_mode_pills` in `src/ui/panels/viewport.rs`, the
bridge is `src/render/viewport/mod.rs`, the frame is in
`crates/runtime/or-renderer/src/viewport.rs`, and the particles are in
[Particle_Preview.md](Particle_Preview.md).

## Modes

| Mode | Draws | Lighting |
|---|---|---|
| Wireframe | the document's edges as lines, no meshes | none |
| Solid | meshes in their builder's colour, white by default (a vehicle with its wheels at its wheel bones) | the inline studio shade |
| Material | meshes with their diffuse texture (a vehicle with its wheels); a GTA VI material that binds no colour map takes no colour and is shaded by its `BumpTex` normal map (`fs_bump`) | the inline studio shade |
| Shaded | every surface of the open drawable, each with the pass its material's effect names, and particles through the passes their rules name | the install's timecycle at the hour and the weather the Time of Day and Weather drivers say (sun or moon, ambient, fog, how wet the world is in the rain and the storm, and the night emissives, off by day and on from the install's hour of evening to its hour of morning, with the camera outdoors), and where the install holds none the engine's constants, a sun over the camera's shoulder and a flat ambient |
| Channel | one vertex channel, unlit | none |

Every mode takes the same forward path and clears to the background colour,
`#2d2d30` (`CLEAR_COLOR`) until the Shaded bar under the pills changes it. A
retired deferred path (G-buffer, lighting, sky, bloom, tonemap, particle
shadows, volumetric fog, decal splats and particle lights, all built in the
renderer's own WGSL) drew the mode that is now Shaded; the game's passes
replaced it, and draw its lights and fog again; what they have no pass wired for, the shadows of particles and decal splats, is a known gap
([Particle_Shader_ABI.md](../reference/Particle_Shader_ABI.md#known-gaps)).

## Forward path

Three passes draw into the sRGB `colour_view`: opaque (the editor's lines with
depth writes, so geometry occludes them, then meshes and particle models, then
the bones over everything), transparent (particles that do not write depth,
trails), and one for particles that do. `MESH_SHADER` has four fragment entry points, `fs_solid`, `fs_channel`,
`fs_material` and `fs_material_blend`; Solid and Material multiply by its `shade`, a
grey key light, camera headlight and ambient. There is no tonemap, bloom, fog or
shadow.

Material draws a texel of alpha 0.999 or more with `fs_material`, which writes depth, and
every other texel in a second sweep of the meshes with `fs_material_blend`, which does not,
in the order the meshes are listed. A translucent shell therefore leaves the surface inside
it visible, as the game's alpha bucket does, and two translucent surfaces blend in list
order rather than by distance.

## Background

`src/state/drivers/world_background.rs` holds the colour the target is cleared
to, and in Shaded mode `draw_shaded_hud_bar` offers it, and the exposure the
effects' evolutions read, under the pills. The colour is kept with the other
viewport preferences as `background`. The Sky checkbox beside it, kept as `sky`, draws
the game's own sky of the time of day where nothing else is drawn, in place of the
colour: a dome about the camera drawn by the pass of the `sky_system` effect that
draws a clear sky, over the pixels the stencil says nothing was drawn at, into the
frame the light is summed in, so that the sun's glow blooms, and the frame is
mapped to the screen there as well as over the drawable. It needs the passes of the
game and a time of day. It draws the disc of the sun, by the effect's `sun` pass blended over the sky by the alpha the sky left, at the size and in the colour of the timecycle. The pass of the sky technique that suits what is up draws the dome, as the game picks it (the sun adds the glow round it, the moon the glow round the moon), and at night the moon's disc, lit as a sphere by the phase of its cycle, and the stars come from `textures/skydome.ytd` of `x64a.rpf`, read with the clip regions; an install without that file shows the night without them. The clouds are cut by the sky pass from noise that the effect's `draw_perlin_noise` pass draws into a map of its own each frame out of `baseperlinnoise3channel` of the same dictionary, with `noise16_p` laid over it and `dither` hiding the steps, and the colours and strengths of the timecycle: they drift with the game's cycle time, which the Time bar's play button runs at an hour a second (the moon's phase and place follow it, as the sun and the sky's colours follow the hour), and stand still while it does not run, and they are not the billboard clouds of the weather, which are another system. The gradient and HDRI backgrounds an earlier version
offered are gone with the deferred path, and a preferences file that names one
reads as the default colour.

The lights an effect gives are drawn in the same frame. A particle rule with a light
behaviour adds a light each update, which the app keeps as the lights of the last
update that ran (`EditorLightSink`), and the viewport draws each as a sphere with the
game's `point` pass that reads a light from a stream of its own, additively over what
the sun lit, so a fire lights the ground around it at night. Only what the G-buffer
holds is lit, so an effect with no drawable under it shows no light: the Ground
checkbox of the bar, kept as `ground`, lays a flat grey ground under an effect in
Shaded for it to land on, and under a model it gives the sun's shadow somewhere to fall: `lay_ground_under` puts the same grey ground a little below the lowest point of a drawable, as wide as three of its longer side, which the shadow pass casts the model onto and the ambient occlusion darkens where it meets it. Lights are the spheres of the game's point pass and the cones of its spot pass (a rule's light type says which, and its axis and its angle shape the cone), at most 64 at once of either, without
shadows or coronas. The ground also gives the sprites something to fade out against: a soft one (a technique of the effect named `_soft`) reads the depth of the picture as the game's pixel stage does, through the size of the picture and the near and far planes it is given, and fades where a surface is close behind it, so a cloud of dust meets the ground softly; the sprites that write depth, and the trails and models, still meet it as they are.

The fog of an effect is drawn in the same frame. A particle rule with a fog volume behaviour adds a sphere of fog each update, which the app keeps as the volumes of the last update that ran (`EditorFogSink`), and the viewport draws each over the frame after what blends, with the game's `vfx_fogvolume` effect: the technique for a sphere, or for an ellipsoid where the rule's scale is not one and its rotation turns it, and the pass for a camera outside the volume or inside it. The pass thins the frame by the volume's density along the way through it, and the depth of the frame ends that way at the nearest surface, so fog stops at the ground and at a model. The colour is the rule's tint by its brightness, and by the sun's light or the day's fog as the rule says, and by the day's ground fog where it asks. The pass holds a fixed number of volumes at once (`MOST` in the fog volume module) and draws no more, and like the lights a volume shows over what the frame holds: the sky and the ground included, and a background alone not.

## Channel mode

Channel renders `MeshVertex::colour` unshaded. `src/render/meshes.rs` packs the
selected channel into it, `src/render/debug_channel.rs` names and colours each
channel, and `draw_channel_selector` offers the choice. Any other mode drops the
choice (`follow_shading_mode`), because Solid and Material multiply by the same
colour, so Channel mode starts again at the first channel the geometry offers.

⚠ Unlit is the contract: `fs_solid`'s lighting term is one the user cannot
subtract by eye, so it turns a debug view into a wrong one.

| Channel | Colour |
|---|---|
| Normal, tangent *n* | `v * 0.5 + 0.5`, so a zero axis is mid-grey rather than black |
| Colour *n* | the channel's own RGB |
| Texcoord *n* | U in red, V in green, clamped to 0..1, so tiled UVs saturate rather than show a false gradient |

- The selector lists the channels the loaded geometry carries, the union of its
  layouts (`offered_by`), not every channel a format can name: over 1,075,524
  vertex declarations in a retail Legacy install, `Tex6`, `Tex7` and `Tangent1`
  occur in none (`openrage-cli channels`, at `2dbc6410`). The pill is hidden
  when the list is empty, because uniform grey reads as a broken viewport.
- Only the selected channel is packed, so `MeshCacheKey` carries it. Widening
  `MeshVertex` to carry every channel costs 2.7× per vertex
  ([R41a](../direction/Roadmap.md#r41a-vertex-and-index-data-as-an-editable-model)).
- Known gap: the cloth and bound mesh builders report no channels, so only
  drawable geometry offers one.

## Shaded: the game's shaders

Shaded draws the open `.ydr`, `.yft` or selected `.ydd` entry with the game's own passes. Its pill is always offered; where no SPIR-V pack is read (`OPENRAGE_GAME_PACK`, else `shader-pack/game` under the configuration folder, where the helper bakes it) its tooltip says to bake one (Automate ▸ Bake game shaders) and it draws as Material, and a restart opens in Material when the pack is gone. The pack is the one `or-shader-tool bake-pack` writes ([Shader_Pipeline_Design.md](../direction/Shader_Pipeline_Design.md#meshes-with-materials)), which the app has the helper bake in the background once a game is set ([how](../direction/Shader_Pipeline_Design.md#baked-from-the-app)). A preferences file written before the mode took this name keeps the key `game`, which reads as Shaded, as the key `shaded` of the deferred mode does.

`src/render/game_scene.rs` extracts the active drawable's most detailed level as an interchange scene with no textures of its own, (a vehicle is drawn with its wheels: it ships one wheel model, the left front's (and a left rear as well for a bike or a truck), in a physics child, and `fragment_wheels` puts it at every wheel bone of the skeleton with the bone's own transform, turned half about the vertical on the far side of the vehicle, the way the game dresses a vehicle with the wheel it ships; the wheel has no shader group and indexes the vehicle's) finds each texture a material names through `texture_resolution::resolve`, as a mesh's diffuse is found, and turns the scene into the surfaces and materials `or_renderer::game_shaders::drawable::surfaces_of` returns. The bridge prepares them once for each `MeshCacheKey` with `ViewportRenderer::prepare_deferred`, which draws as the game does: what is opaque is written by each effect's `deferred_draw` pass to four G-buffer targets and to the viewport's depth and stencil (the stencil holds the material, and a pixel nothing drew keeps the value the lighting skips); where the sun shines the surfaces are drawn from it as depth into one tile of the game's cascade map, by the depth-only passes of its `cascadeshadows` effect, and the game's `cascadeshadows_rendering` pass multiplies what reaches each pixel into the self-shadow in the alpha of the G-buffer; the game's `ssao` passes then darken the ambient scales of the G-buffer where other surfaces stand close around a pixel (the downscaled depth, the sampling, two blurs that keep the edges of depth, and the upscale that multiplies them in, at the strength of the day's `ssao_inten`), and the game's own `directional` pass reads them and lights the frame, by the sun where it shines and by the ambient light alone where it does not; what blends is drawn into the lit frame with the effect's forward pass (and marks the stencil as what is opaque does, except for what is seen through (the glass of a window or a vehicle, the blur of a rotor) where something opaque is drawn, since a pane over nothing that marked it would be mapped to the screen as black in place of letting the background show through, and the rotor blur of a helicopter would be a black star), the bright part of the frame is blurred at four sizes by the game's own bloom passes and added back to it (and, round what was drawn, to the background), and the frame is mapped by the game's `postfx` pass (`pp_simple`) to a target of its own, which the game's anti-aliasing pass of the same effect (`pp_AA`, FXAA, reading the luminance the mapping leaves in alpha) draws to the screen, and the lines and the models are drawn over that. Where the pack has no such passes, or the device refuses them, the bridge prepares a `MeshScene` instead (`ViewportRenderer::game_target`: its colour, and its reversed depth, one at the near plane), and each surface is lit by its own forward pass. A surface whose effect the pack lacks, or that has no pass the route draws, is left out and listed in the pill's tooltip. Without a scene, as when no drawable is open, the mode draws as Material does, rather than a blank viewport.

What a scene costs to make and to draw is held down by four things it keeps (#804, #806 to #809 and #811). A buffer of constants is planned once for the names the engine sets, so a frame copies numbers to places found before and writes the buffer only where its bytes changed (`Uniform::write`); and the surfaces whose constants come to the same bytes before the engine's values (the layout, the effect's defaults and the material's vectors) share one buffer, which a scene writes once for all of them. A texture is uploaded once for each distinct set of texels, found by a sample of them and confirmed by comparing them all, with the levels below its first made a row at a time across the cores, and the pipeline cache keeps those uploads from a scene to the next, letting go of the ones the next scene does not name, so an edit that leaves a texture alone does not upload it again; the bridge likewise keeps the first levels the textures of the last scene were decoded to (`DecodedTextures`). A vertex stage's file is read and its vertex layout worked out when its pipeline is built, and kept with the pipeline. Each of these has a test that fails without it, and the renderer's own cost is measured by timing `render` and the scene's preparation in a release build, which is how the figures in those pull requests were taken.

Water is drawn too, with the game's own water pass and stand-ins for part of what the game gives it. Its effects name their passes by number (the one that draws with the environment is `alt4_draw` for the shallow and pool water, and the river, ocean and fountain effects keep `draw`), it has no deferred pass, so it is lit by its own over the lit frame, and the pass writes its whole colour with a nought alpha, which is why water replaces what it covers where every other blended surface blends (an alpha blend of it draws nothing, and an alpha written over the frame whitens it through the bloom). The game gives the water its colour before the surface is drawn: its fog pass (the `imposter_draw` technique, which the river, shallow, pool and fountain effects name for it) writes the fog colour of the mesh, a texture the drawable carries, into the lit frame where the water lies in front of what is opaque, scaled by the light the water takes (the ambient light and the sun by its height, `water_light`), through a blend of the constant the pass sets, and a second colour it writes goes to a target nothing reads. What the surface pass reads behind it is then real: the lit frame, with that colour laid in, is copied just before what blends is drawn (a texture of the size of the frame, made again with the targets, so the scene that holds water is prepared again when the frame is resized: `deferred_needs_prepare`), and the distance of each opaque pixel along the view, in metres, is the half-size depth the ambient occlusion's first pass makes, which that pass makes for water even where the day gives no occlusion (`water_reads_the_frame_behind_it_and_how_far_that_lies_and_is_prepared_again_with_the_targets` reads both back). What it reflects is the map of the sky the frame is lit by (the water's sun and ambient colours are the day's), how thick the water lies is the second colour the fog pass writes, kept in an 8-bit target the surface pass reads, and how much a ray reflects is the curve the game draws once for it, Fresnel's for air and water (`blend_row`), so the water is the colour of its fog with the sky over it as the angle grows, and the ripples and the foam are the maps the install's shader library holds for them (`waterbump`, `waterbump2` and `waterfoam` of `graphics.ytd`, read with the sky's dictionary in one opening of its archive and bound by the names the effects give their samplers), and the game's water data are not read, so it ripples as the maps say and does not reflect what stands round it; the planar reflection the game draws with each effect's `waterreflection_draw` pass is not made.

A skinned surface is drawn in the pose the animation holds, as the game draws it: each surface carries the bone of its drawable's skeleton every weight is of, which is bound to the bone of the skeleton the clip is bound to that has the same tag (`rebind_joints`, a bone with none keeping its index), the renderer gives a scene the table of the bones its surfaces are weighted to (`bone_palette`, at most 255, the room the game's buffer has) as the vertex's blend indices (a float input is a normalized byte the shader multiplies by 255, so the slot is written divided by 255), and writes the skin matrices of the pose to `gBoneMtx` in that order each frame (`set_skin`, which leaves the dirty flag alone: that one says the colour texture was made again, and a clip moves the pose every frame). A pose at rest draws what no pose does, and the Animation workspace draws the drawables its preview draws through the same passes as the Model workspace draws the active document's. The GPU test of the skin (`a_skinned_surface_is_drawn_in_the_pose_it_is_given_through_the_bones_it_is_weighted_to`) draws a floor in a pose and the same floor with its corners turned and moved on the CPU, and the two frames are equal.

Particles take the same pack: sprites, trails and models draw with the `ptfx_sprite`, `ptfx_trail` and `ptfx_model` passes their rules name, in this mode with the blend modes the retail engine has (`retail_parity`).

This reopens what was cut here: a mode through the game's shaders needed a matched vertex and pixel shader for each material, and a pack baked with the first chain had 919 of 3,676 vertex blobs and 265 of 4,992 pixel blobs (`a3961165`). Through the SPIR-V pack, of 3,000 `.ydr` and `.yft` entries of a Legacy install drawn headless on a Metal device, 2,816 show something and 10,924 of their 10,941 surfaces draw (at `d270f330`; [measured](../direction/Shader_Pipeline_Design.md#meshes-with-materials)). What it does not do: shade as the game does, since the only shadow is the drawable's own, cast by the sun into one tile (no cascades, no ground, no softening, and every opaque surface casts whole, a cutout's holes included), occlusion of the ambient light only by the game's screen-space pass, reflections of the day's sky alone (its atmosphere and the glow of its sun, as the game's sky shader blends them, with no clouds and no world) and no lights but the sun, and the engine constants other than the day's light are defaults, the post-processing is the game's simple pass alone, which maps the light to the screen by the filmic curve and the colour correction of the day, at an exposure the curve of the visual settings gives for the light of the moment, where the game measures the average luminance of the frame it has drawn, and there is no depth of field and no anti-aliasing beyond that pass, and the draw buckets are read only as far as three kinds: a surface in an alpha, decal or displacement bucket blends over what is behind it, after what is opaque, without writing depth; one in the cutout bucket takes the effect's own clipping technique where it has one; the rest draw opaque, whatever state the game's own code sets for them.

## The dressed ped

The Wardrobe's dressed ped has no document, so it takes the modes through `Draw` in its own tools
(`ViewMode`, `src/ui/panels/wardrobe/tools.rs`) and not through the pills: textures is Material,
shape is Solid, edges is Wireframe, and the normal of each vertex and its first texture coordinates
are Channel with the colours packed into the vertices by `refresh_dressed_as` (the channel is part
of the key of what is uploaded, so a change of view builds the draws again). Wireframe draws each
edge of the ped once, from the vertices as the last pose wrote them, so a clip moves the edges as it
moves the ped. Shaded is not offered: the game's passes belong to a document's materials. The
self-check draws each view and the bones through the viewport.

## Out of scope

A sky or an environment image behind the model; the game's post-processing; a
background per viewport (there is one viewport panel
and one `world_background`); a colour management toggle for the background,
whose colour is taken as linear.
