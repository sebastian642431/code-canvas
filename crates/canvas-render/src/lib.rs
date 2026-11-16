//! `canvas-render`: draws a [`canvas_core::Layout`] with wgpu.
//!
//! The canvas is drawn in a single render pass, into an encoder owned by the
//! caller (who then draws its UI on top and submits):
//!
//! 1. frames (directory boxes, file cards) — static buffer, range-culled
//! 2. `under` rectangles (symbol bodies, outline regions)
//! 3. code bars at the current [`Lod`] — static (blocks) or per-frame (lines, tokens)
//! 4. text: code chunks, file and folder names
//! 5. relation curves
//! 6. `over` rectangles (hover, selection, usage markers)
//! 7. chips: screen-space rectangles and their labels (symbol names)

pub mod curve;
mod gpu_util;
pub mod scene;
pub mod text;
pub mod theme;

use bytemuck::{Pod, Zeroable};
use canvas_core::layout::{NodeKind, FOLD_GAP, FOLD_LINES};
use canvas_core::{Layout, Rect, Repo, TokenCache, LINE_H};

pub use curve::{push_curve, CurveVertex};
pub use scene::{CodeInstance, Lod, RectInstance, SceneData, SceneGpu};
pub use text::{CodeText, Label, TextLayer};

use gpu_util::DynBuffer;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct CameraUniform {
    offset: [f32; 2],
    zoom: f32,
    alpha: f32,
    viewport: [f32; 2],
    _pad: [f32; 2],
}

/// What the camera looks at. `offset` is the world point at the top-left of
/// the screen; `zoom` is physical pixels per world unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub offset: [f32; 2],
    pub zoom: f32,
    /// Physical pixels.
    pub viewport: [f32; 2],
}

impl View {
    pub fn world_rect(&self) -> Rect {
        Rect::new(self.offset[0], self.offset[1], self.viewport[0] / self.zoom, self.viewport[1] / self.zoom)
    }

    pub fn to_screen(&self, p: [f32; 2]) -> [f32; 2] {
        [(p[0] - self.offset[0]) * self.zoom, (p[1] - self.offset[1]) * self.zoom]
    }

    pub fn to_world(&self, s: [f32; 2]) -> [f32; 2] {
        [s[0] / self.zoom + self.offset[0], s[1] / self.zoom + self.offset[1]]
    }
}

/// The page being drawn this frame.
pub struct ScenePass<'a> {
    pub page: u32,
    pub gpu: &'a mut SceneGpu,
    pub repo: &'a Repo,
    pub layout: &'a Layout,
    /// Tokenized chunks of this page (tokens are produced on demand).
    pub tokens: &'a mut TokenCache,
    /// Screen area the canvas may draw text into (left, top, right, bottom),
    /// so code never bleeds over the UI bars.
    pub clip: [f32; 4],
}

/// Everything drawn on top of (or just under) the code this frame.
#[derive(Default)]
pub struct Overlay<'a> {
    /// World rectangles under the code (symbol bodies, outline regions).
    pub under: &'a [RectInstance],
    /// File and folder names, in screen space (drawn with the code text).
    pub labels: &'a [Label<'a>],
    /// World rectangles above the code (hover, selection, usage markers).
    pub over: &'a [RectInstance],
    /// Screen-space rectangles above everything (chip backgrounds).
    pub chips: &'a [RectInstance],
    /// Text above the chips.
    pub chip_labels: &'a [Label<'a>],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub lod: Option<Lod>,
    pub code_instances: u32,
    pub frame_instances: u32,
    pub draw_calls: u32,
    pub text_files: u32,
    pub chunks_shaped: u32,
    pub cached_chunks: u32,
    pub text_pending: bool,
    /// Chunks in the page's token cache / tokenized this frame.
    pub token_chunks: u32,
    pub tokenized: u32,
}

pub struct Renderer {
    rect_pipeline: wgpu::RenderPipeline,
    code_pipeline: wgpu::RenderPipeline,
    curve_pipeline: wgpu::RenderPipeline,
    pub origins_layout: wgpu::BindGroupLayout,
    world_cam: wgpu::Buffer,
    screen_cam: wgpu::Buffer,
    world_bind: wgpu::BindGroup,
    screen_bind: wgpu::BindGroup,
    under: DynBuffer,
    over: DynBuffer,
    chips: DynBuffer,
    curves: DynBuffer,
    curve_vertices: u32,
    pub text: TextLayer,
}

fn instance_layout<'a>(stride: u64, attrs: &'a [wgpu::VertexAttribute]) -> wgpu::VertexBufferLayout<'a> {
    wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Instance, attributes: attrs }
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        use wgpu::util::DeviceExt;

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let origins_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("origins"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let cam_buf = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<CameraUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let world_cam = cam_buf("world camera");
        let screen_cam = cam_buf("screen camera");
        let palette: Vec<[f32; 4]> = theme::TOKEN_COLORS.iter().map(|&c| theme::to_f32(c)).collect();
        let palette_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("palette"),
            contents: bytemuck::cast_slice(&palette),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = |cam: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("globals"),
                layout: &globals_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: cam.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: palette_buf.as_entire_binding() },
                ],
            })
        };
        let world_bind = bind(&world_cam);
        let screen_bind = bind(&screen_cam);

        let blend_target = [Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let strip = wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() };

        let rect_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rect.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/rect.wgsl").into()),
        });
        let rect_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rect"),
            bind_group_layouts: &[Some(&globals_layout)],
            ..Default::default()
        });
        let rect_attrs = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Uint32, 3 => Uint32, 4 => Float32, 5 => Float32
        ];
        let rect_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rect"),
            layout: Some(&rect_layout),
            vertex: wgpu::VertexState {
                module: &rect_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(instance_layout(std::mem::size_of::<RectInstance>() as u64, &rect_attrs))],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &rect_shader,
                entry_point: Some("fs_main"),
                targets: &blend_target,
                compilation_options: Default::default(),
            }),
            primitive: strip,
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
            multiview_mask: None,
        });

        let code_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("code.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/code.wgsl").into()),
        });
        let code_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("code"),
            bind_group_layouts: &[Some(&globals_layout), Some(&origins_layout)],
            ..Default::default()
        });
        let code_attrs = wgpu::vertex_attr_array![0 => Uint32, 1 => Uint32, 2 => Uint32];
        let code_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("code"),
            layout: Some(&code_layout),
            vertex: wgpu::VertexState {
                module: &code_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(instance_layout(std::mem::size_of::<CodeInstance>() as u64, &code_attrs))],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &code_shader,
                entry_point: Some("fs_main"),
                targets: &blend_target,
                compilation_options: Default::default(),
            }),
            primitive: strip,
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
            multiview_mask: None,
        });

        let curve_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("curve.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/curve.wgsl").into()),
        });
        let curve_attrs =
            wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32, 3 => Float32, 4 => Uint32];
        let curve_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("curve"),
            layout: Some(&rect_layout),
            vertex: wgpu::VertexState {
                module: &curve_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<CurveVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &curve_attrs,
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &curve_shader,
                entry_point: Some("fs_main"),
                targets: &blend_target,
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
            multiview_mask: None,
        });

        Self {
            rect_pipeline,
            code_pipeline,
            curve_pipeline,
            origins_layout,
            world_cam,
            screen_cam,
            world_bind,
            screen_bind,
            under: DynBuffer::new(device, "under rects", wgpu::BufferUsages::VERTEX),
            over: DynBuffer::new(device, "over rects", wgpu::BufferUsages::VERTEX),
            chips: DynBuffer::new(device, "chip rects", wgpu::BufferUsages::VERTEX),
            curves: DynBuffer::new(device, "curves", wgpu::BufferUsages::VERTEX),
            curve_vertices: 0,
            text: TextLayer::new(device, queue, format),
        }
    }

    /// Upload a page's static geometry (built off-thread with [`SceneData::build`]).
    pub fn create_scene(&self, device: &wgpu::Device, name: &str, data: SceneData) -> SceneGpu {
        SceneGpu::new(device, &self.origins_layout, name, data)
    }

    /// Record the canvas pass into `encoder` (clearing `target`). The caller
    /// submits the encoder and then calls [`Renderer::end_frame`].
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        view: View,
        mut scene: Option<ScenePass>,
        overlay: &Overlay,
    ) -> FrameStats {
        let mut stats = FrameStats::default();
        let lod = Lod::for_zoom(view.zoom);
        let world_rect = view.world_rect();

        let code_alpha = match lod {
            Lod::Blocks => 0.55,
            Lod::Lines => 0.75,
            _ => 0.9,
        };
        queue.write_buffer(
            &self.world_cam,
            0,
            bytemuck::bytes_of(&CameraUniform {
                offset: view.offset,
                zoom: view.zoom,
                alpha: code_alpha,
                viewport: view.viewport,
                _pad: [0.0; 2],
            }),
        );
        queue.write_buffer(
            &self.screen_cam,
            0,
            bytemuck::bytes_of(&CameraUniform {
                offset: [0.0, 0.0],
                zoom: 1.0,
                alpha: 1.0,
                viewport: view.viewport,
                _pad: [0.0; 2],
            }),
        );

        // Geometry + text for the page.
        let mut code_texts: Vec<CodeText> = Vec::new();
        if let Some(sc) = scene.as_mut() {
            stats.lod = Some(lod);
            sc.tokens.begin_frame();
            stats.code_instances = sc.gpu.prepare(device, queue, sc.repo, sc.layout, sc.tokens, &world_rect, lod);
            stats.frame_instances = sc.gpu.frame_ranges.iter().map(|r| r.end - r.start).sum();
            if lod == Lod::Text {
                let clip = sc.clip;
                sc.layout.visit_visible(&world_rect, |_, n| {
                    if let NodeKind::File(fi) = n.kind {
                        let f = &sc.repo.files[fi];
                        let o = n.code_origin();
                        let tl = view.to_screen([n.rect.x, n.rect.y]);
                        let br = view.to_screen([n.rect.right(), n.rect.bottom()]);
                        for (col, lines) in n.visible_lines(&world_rect, f.line_count()) {
                            // Folded column `col` shows lines from col * FOLD_LINES,
                            // shifted right by col * col_stride.
                            let cx = o[0] + col as f32 * n.col_stride;
                            let top_line = (col * FOLD_LINES) as f32 * LINE_H;
                            let so = view.to_screen([cx, o[1] - top_line]);
                            let col_l = view.to_screen([cx, 0.0])[0];
                            let col_r = view.to_screen([cx + n.col_stride - FOLD_GAP, 0.0])[0];
                            code_texts.push(CodeText {
                                page: sc.page,
                                file_index: fi,
                                file: f,
                                lines,
                                left: so[0],
                                top: so[1],
                                zoom: view.zoom,
                                clip: [
                                    tl[0].max(col_l - 2.0).max(clip[0]) as i32,
                                    tl[1].max(clip[1]) as i32,
                                    br[0].min(col_r + 2.0).min(clip[2]) as i32,
                                    br[1].min(clip[3]) as i32,
                                ],
                            });
                        }
                    }
                    true
                });
                stats.text_files = code_texts.len() as u32;
            }
        }

        self.under.write(device, queue, bytemuck::cast_slice(overlay.under));
        self.over.write(device, queue, bytemuck::cast_slice(overlay.over));
        self.chips.write(device, queue, bytemuck::cast_slice(overlay.chips));
        self.text.prepare(
            device,
            queue,
            [view.viewport[0] as u32, view.viewport[1] as u32],
            &code_texts,
            scene.as_mut().map(|sc| &mut *sc.tokens),
            overlay.labels,
            overlay.chip_labels,
        );
        stats.chunks_shaped = self.text.chunks_shaped_last_frame as u32;
        stats.text_pending = self.text.pending;
        stats.cached_chunks = self.text.cached_chunks() as u32;
        if let Some(sc) = scene.as_ref() {
            stats.token_chunks = sc.tokens.len() as u32;
            stats.tokenized = sc.tokens.tokenized_this_frame as u32;
        }

        {
            let bg = theme::BACKGROUND;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("canvas"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: bg[0], g: bg[1], b: bg[2], a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            let rect_pipeline = &self.rect_pipeline;
            let mut extra_draws = 0u32;
            let mut rects = |pass: &mut wgpu::RenderPass, bind: &wgpu::BindGroup, buf: &DynBuffer, n: usize| {
                if n > 0 {
                    pass.set_pipeline(rect_pipeline);
                    pass.set_bind_group(0, bind, &[]);
                    pass.set_vertex_buffer(0, buf.buf.slice(..));
                    pass.draw(0..4, 0..n as u32);
                    extra_draws += 1;
                }
            };

            if let Some(sc) = scene.as_ref() {
                let g = &*sc.gpu;
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_bind_group(0, &self.world_bind, &[]);
                pass.set_vertex_buffer(0, g.frames.slice(..));
                for r in &g.frame_ranges {
                    pass.draw(0..4, r.clone());
                }
                rects(&mut pass, &self.world_bind, &self.under, overlay.under.len());
                if !g.code_ranges.is_empty() {
                    pass.set_pipeline(&self.code_pipeline);
                    pass.set_bind_group(0, &self.world_bind, &[]);
                    pass.set_bind_group(1, &g.origins_bind, &[]);
                    let buf = if lod == Lod::Blocks { &g.blocks } else { &g.dynamic.buf };
                    pass.set_vertex_buffer(0, buf.slice(..));
                    for r in &g.code_ranges {
                        pass.draw(0..4, r.clone());
                    }
                }
                stats.draw_calls += (g.frame_ranges.len() + g.code_ranges.len()) as u32;
            }
            self.text.render_world(&mut pass);
            if self.curve_vertices > 0 {
                pass.set_pipeline(&self.curve_pipeline);
                pass.set_bind_group(0, &self.world_bind, &[]);
                pass.set_vertex_buffer(0, self.curves.buf.slice(..));
                pass.draw(0..self.curve_vertices, 0..1);
                stats.draw_calls += 1;
            }
            rects(&mut pass, &self.world_bind, &self.over, overlay.over.len());
            rects(&mut pass, &self.screen_bind, &self.chips, overlay.chips.len());
            self.text.render_ui(&mut pass);
            stats.draw_calls += extra_draws;
        }
        stats
    }

    /// Replace the relation curves (world space; see [`push_curve`]). They
    /// stay on the GPU until replaced, so call this only when they change.
    pub fn set_curves(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, vertices: &[CurveVertex]) {
        self.curves.write(device, queue, bytemuck::cast_slice(vertices));
        self.curve_vertices = vertices.len() as u32;
    }

    /// Call after the frame's encoder was submitted.
    pub fn end_frame(&mut self) {
        self.text.end_frame();
    }
}
