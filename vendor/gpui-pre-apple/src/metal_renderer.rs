// Copyright 2022 - 2025 Zed Industries, Inc.
// SPDX-License-Identifier: Apache-2.0
// Modified for BenCode: MonoCode's backdrop blur (`backdrop_blur_marker`),
// cutouts for native views (`hole_punch_marker`) and `trim`. See ../NOTICE.
use crate::metal_atlas::MetalAtlas;
use anyhow::{Context as _, Result};
use block2::RcBlock;
use gpui::{
    AtlasTextureId, Bounds, ContentMask, Corners, DevicePixels, Hsla, PaintSurface, PaintedPath,
    Pixels,
    PolychromeSprite, PrimitiveBatch, Quad, ScaledPixels, Scene, Shadow, Size, quad_depth, quad_depth_scale, size,
};
#[cfg(any(test, feature = "bench-support", feature = "test-support"))]
use image::RgbaImage;
use objc2::{
    Message, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, ProtocolObject},
};

// MPSImageGaussianBlur for frosted glass.
#[link(name = "MetalPerformanceShaders", kind = "framework")]
unsafe extern "C" {}

use objc2_core_foundation::{CFRetained, CGSize};
use objc2_core_graphics::{CGColorSpace, kCGColorSpaceDisplayP3};
use objc2_core_video::{
    CVMetalTexture, CVMetalTextureCache, CVMetalTextureGetTexture, CVPixelBufferGetHeight,
    CVPixelBufferGetHeightOfPlane, CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth,
    CVPixelBufferGetWidthOfPlane, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange, kCVReturnSuccess,
};
use objc2_foundation::{NSRange, NSString};
#[cfg(target_os = "macos")]
use objc2_metal::MTLCopyAllDevices;
use objc2_metal::{
    MTLBlendFactor, MTLBlendOperation, MTLBlitCommandEncoder, MTLBuffer, MTLClearColor, MTLCommandBuffer,
    MTLCommandEncoder, MTLCommandQueue, MTLCompareFunction, MTLCreateSystemDefaultDevice,
    MTLDepthStencilDescriptor, MTLDepthStencilState, MTLDevice, MTLDrawable, MTLGPUFamily,
    MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLRenderCommandEncoder,
    MTLRenderPassDescriptor, MTLRenderPipelineDescriptor, MTLRenderPipelineState,
    MTLResourceOptions, MTLSize, MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor,
    MTLTextureUsage, MTLViewport,
};
#[cfg(any(test, feature = "bench-support", feature = "test-support"))]
use objc2_metal::MTLRegion;
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use parking_lot::Mutex;

use std::{
    cell::Cell,
    mem,
    mem::MaybeUninit,
    ops::Range,
    ptr, slice,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[cfg(not(feature = "runtime_shaders"))]
const SHADERS_METALLIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/shaders.metallib"));
#[cfg(feature = "runtime_shaders")]
const SHADERS_SOURCE_FILE: &str = include_str!(concat!(env!("OUT_DIR"), "/stitched_shaders.metal"));
/// Metal requires the offset a buffer is bound at to be 256-byte aligned.
const INSTANCE_BUFFER_ALIGNMENT: usize = 256;
const MAX_INSTANCE_BUFFER_SIZE: usize = 256 * 1024 * 1024;
/// The backdrop's scratch textures go after this long without a frosted quad.
const SCRATCH_IDLE: Duration = Duration::from_secs(2);
/// The instance buffers' size before any frame outgrew it.
const DEFAULT_INSTANCE_BUFFER_SIZE: usize = 2 * 1024 * 1024;

/// Bumped by [`trim`]; each renderer gives back what it can on its next
/// frame (BenCode, after MonoCode's `trim`).
static TRIM_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Gives GPU memory back once the app is quiet: empties every window's
/// sprite atlas now and has each renderer's next frame drop the instance
/// buffers back to their first size. Call it between frames and refresh
/// every window right after (`App::refresh_windows`), so each frame paints
/// afresh and uploads only what it draws.
pub fn trim() {
    crate::metal_atlas::clear_all();
    TRIM_GENERATION.fetch_add(1, Ordering::Relaxed);
}

pub type Context = Arc<Mutex<InstanceBufferPool>>;
pub type Renderer = MetalRenderer;

pub fn new_renderer(
    context: self::Context,
    transparent: bool,
    use_display_p3: bool,
) -> Result<Renderer> {
    MetalRenderer::new(context, transparent, use_display_p3)
}

pub struct InstanceBufferPool {
    buffer_size: usize,
    buffers: Vec<InstanceBuffer>,
}

impl Default for InstanceBufferPool {
    fn default() -> Self {
        Self {
            buffer_size: DEFAULT_INSTANCE_BUFFER_SIZE,
            buffers: Vec::new(),
        }
    }
}

struct InstanceBuffer {
    metal_buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
    size: usize,
}

// SAFETY: Each buffer has one CPU writer and returns to the mutex-protected pool only after GPU completion.
unsafe impl Send for InstanceBuffer {}

impl InstanceBufferPool {
    pub(crate) fn reset(&mut self, buffer_size: usize) {
        self.buffer_size = buffer_size;
        self.buffers.clear();
    }

    fn acquire(
        &mut self,
        device: &ProtocolObject<dyn MTLDevice>,
        unified_memory: bool,
    ) -> Result<InstanceBuffer> {
        if let Some(buffer) = self.buffers.pop() {
            return Ok(buffer);
        }
        let options = if unified_memory {
            MTLResourceOptions::StorageModeShared
                // Buffers are write only which can benefit from the combined cache
                // https://developer.apple.com/documentation/metal/mtlresourceoptions/cpucachemodewritecombined
                | MTLResourceOptions::CPUCacheModeWriteCombined
        } else {
            MTLResourceOptions::StorageModeManaged
        };
        let buffer = device
            .newBufferWithLength_options(self.buffer_size, options)
            .with_context(|| format!("allocating {}-byte instance buffer", self.buffer_size))?;
        Ok(InstanceBuffer {
            metal_buffer: buffer,
            size: self.buffer_size,
        })
    }

    fn release(&mut self, buffer: InstanceBuffer) {
        if buffer.size == self.buffer_size {
            self.buffers.push(buffer)
        }
    }
}

enum DisplayLinkMode {
    Detached,
    Attached {
        drawable: Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
    },
}

pub struct MetalRenderer {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    layer: Option<Retained<CAMetalLayer>>,
    is_apple_gpu: bool,
    is_unified_memory: bool,
    presents_with_transaction: bool,
    display_link_mode: DisplayLinkMode,
    /// For headless rendering, tracks whether output should be opaque
    opaque: bool,
    /// Whether the last scene drawn to the layer let native views show through it.
    shows_native_views: bool,
    command_queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    path_tiles_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    shadows_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    quads_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    opaque_quads_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    underlines_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    monochrome_sprites_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    polychrome_sprites_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    surfaces_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    native_views_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    clear_quads_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    /// Built by the first frame that draws a chat wallpaper (`chat_background_corners`).
    chat_background_pipeline_state: Option<Retained<ProtocolObject<dyn MTLRenderPipelineState>>>,
    library: Retained<ProtocolObject<dyn MTLLibrary>>,
    backdrop_composite_pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    /// `MPSImageGaussianBlur` kernels by sigma, most recently used last.
    backdrop_kernels: Vec<(f32, Retained<AnyObject>)>,
    /// The snapshot and blurred copy of the last backdrop region, reused
    /// while frames keep frosting a region of the same size.
    backdrop_scratch: Option<(
        Retained<ProtocolObject<dyn MTLTexture>>,
        Retained<ProtocolObject<dyn MTLTexture>>,
    )>,
    backdrop_used: Option<Instant>,
    /// The [`trim`] this renderer last followed.
    trim_generation: u64,
    depth_write_state: Retained<ProtocolObject<dyn MTLDepthStencilState>>,
    depth_test_state: Retained<ProtocolObject<dyn MTLDepthStencilState>>,
    depth_texture: Option<Retained<ProtocolObject<dyn MTLTexture>>>,
    instance_buffer_pool: Arc<Mutex<InstanceBufferPool>>,
    sprite_atlas: Arc<MetalAtlas>,
    core_video_texture_cache: CFRetained<CVMetalTextureCache>,
    /// Offscreen render target reused across `render_scene` calls when
    /// rendering headlessly without reading pixels back.
    #[cfg(any(test, feature = "bench-support", feature = "test-support"))]
    headless_render_target: Option<Retained<ProtocolObject<dyn MTLTexture>>>,
}

impl MetalRenderer {
    /// Creates a new MetalRenderer with a CAMetalLayer for window-based rendering.
    pub fn new(
        instance_buffer_pool: Arc<Mutex<InstanceBufferPool>>,
        transparent: bool,
        use_display_p3: bool,
    ) -> Result<Self> {
        let device = Self::create_device()?;
        let layer = CAMetalLayer::new();
        Self::configure_layer(&layer, &device, transparent, use_display_p3);
        Self::new_internal(device, Some(layer), !transparent, instance_buffer_pool)
    }

    /// Creates a renderer for a CAMetalLayer owned by a platform view, such as
    /// the backing layer UIKit creates for a view whose `layerClass` is
    /// `CAMetalLayer`. The renderer retains the layer.
    pub fn from_layer(
        instance_buffer_pool: Arc<Mutex<InstanceBufferPool>>,
        layer: &CAMetalLayer,
        transparent: bool,
    ) -> Result<Self> {
        let device = Self::create_device()?;
        let layer = layer.retain();
        Self::configure_layer(&layer, &device, transparent, false);
        Self::new_internal(device, Some(layer), !transparent, instance_buffer_pool)
    }

    fn configure_layer(
        layer: &CAMetalLayer,
        device: &ProtocolObject<dyn MTLDevice>,
        transparent: bool,
        use_display_p3: bool,
    ) {
        layer.setDevice(Some(device));
        layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        if use_display_p3 {
            // SAFETY: The constant is extern, from Core Graphics
            let display_p3_name = unsafe { kCGColorSpaceDisplayP3 };
            if let Some(display_p3_color_space) = CGColorSpace::with_name(Some(display_p3_name)) {
                layer.setColorspace(Some(&display_p3_color_space));
            } else {
                log::error!("unable to create the Display P3 color space");
            }
        }
        // Support direct-to-display rendering if the window is not transparent
        // https://developer.apple.com/documentation/metal/managing-your-game-window-for-metal-in-macos
        layer.setOpaque(!transparent);
        layer.setMaximumDrawableCount(3);
        // BenCode: backdrop blur copies already drawn pixels out of the
        // drawable, so it must stay readable in every build, not only for
        // visual tests and UI automation screenshots.
        layer.setFramebufferOnly(false);
        layer.setAllowsNextDrawableTimeout(false);
        layer.setNeedsDisplayOnBoundsChange(true);
        // UIKit sizes a view's backing layer itself; only AppKit-hosted
        // layers need to track their superlayer's bounds.
        #[cfg(target_os = "macos")]
        layer.setAutoresizingMask(
            objc2_quartz_core::CAAutoresizingMask::LayerWidthSizable
                | objc2_quartz_core::CAAutoresizingMask::LayerHeightSizable,
        );
    }

    /// Creates a new headless MetalRenderer for offscreen rendering without a window.
    ///
    /// This renderer can render scenes to images without requiring a CAMetalLayer,
    /// window, or AppKit. Use `render_scene_to_image()` to render scenes.
    #[cfg(any(test, feature = "bench-support", feature = "test-support"))]
    pub fn new_headless(instance_buffer_pool: Arc<Mutex<InstanceBufferPool>>) -> Result<Self> {
        let device = Self::create_device()?;
        Self::new_internal(device, None, true, instance_buffer_pool)
    }

    #[cfg(target_os = "macos")]
    fn create_device() -> Result<Retained<ProtocolObject<dyn MTLDevice>>> {
        // Prefer low‐power integrated GPUs on Intel Mac. On Apple
        // Silicon, there is only ever one GPU, so this is equivalent to
        // `metal::Device::system_default()`.
        if let Some(device) = MTLCopyAllDevices()
            .iter()
            .min_by_key(|device| (device.isRemovable(), !device.isLowPower()))
        {
            Ok(device)
        } else {
            // For some reason `all()` can return an empty list, see https://github.com/zed-industries/zed/issues/37689
            // In that case, we fall back to the system default device.
            log::error!(
                "Unable to enumerate Metal devices; attempting to use system default device"
            );
            Self::system_default_device()
        }
    }

    #[cfg(target_os = "ios")]
    fn create_device() -> Result<Retained<ProtocolObject<dyn MTLDevice>>> {
        Self::system_default_device()
    }

    fn system_default_device() -> Result<Retained<ProtocolObject<dyn MTLDevice>>> {
        MTLCreateSystemDefaultDevice().context("accessing a compatible Metal device")
    }

    fn new_internal(
        device: Retained<ProtocolObject<dyn MTLDevice>>,
        layer: Option<Retained<CAMetalLayer>>,
        opaque: bool,
        instance_buffer_pool: Arc<Mutex<InstanceBufferPool>>,
    ) -> Result<Self> {
        #[cfg(feature = "runtime_shaders")]
        let library = device
            .newLibraryWithSource_options_error(
                &NSString::from_str(SHADERS_SOURCE_FILE),
                Some(&objc2_metal::MTLCompileOptions::new()),
            )
            .context("building Metal library")?;
        #[cfg(not(feature = "runtime_shaders"))]
        let library = device
            .newLibraryWithData_error(&dispatch2::DispatchData::from_static_bytes(
                SHADERS_METALLIB,
            ))
            .context("loading Metal library")?;

        // Shared memory can be used only if CPU and GPU share the same memory space.
        // https://developer.apple.com/documentation/metal/setting-resource-storage-modes
        // iOS does not support managed resources. Its simulator may report a
        // non-unified host GPU even though resources must still use shared storage.
        let is_unified_memory = cfg!(target_os = "ios") || device.hasUnifiedMemory();
        // Only Apple GPUs can create shared-storage textures on macOS.
        // https://developer.apple.com/documentation/metal/mtlgpufamily
        let is_apple_gpu = device.supportsFamily(MTLGPUFamily::Apple1);

        let path_tiles_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "path_tiles",
            "path_tile_vertex",
            "path_tile_fragment",
            MTLPixelFormat::BGRA8Unorm,
            true,
        )?;
        let shadows_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "shadows",
            "shadow_vertex",
            "shadow_fragment",
            MTLPixelFormat::BGRA8Unorm,
            true,
        )?;
        let quads_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "quads",
            "quad_vertex",
            "quad_fragment",
            MTLPixelFormat::BGRA8Unorm,
            true,
        )?;
        let opaque_quads_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "opaque_quads",
            "opaque_quad_vertex",
            "opaque_quad_fragment",
            MTLPixelFormat::BGRA8Unorm,
            false,
        )?;
        let underlines_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "underlines",
            "underline_vertex",
            "underline_fragment",
            MTLPixelFormat::BGRA8Unorm,
            true,
        )?;
        let monochrome_sprites_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "monochrome_sprites",
            "monochrome_sprite_vertex",
            "monochrome_sprite_fragment",
            MTLPixelFormat::BGRA8Unorm,
            true,
        )?;
        let polychrome_sprites_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "polychrome_sprites",
            "polychrome_sprite_vertex",
            "polychrome_sprite_fragment",
            MTLPixelFormat::BGRA8Unorm,
            true,
        )?;
        let surfaces_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "surfaces",
            "surface_vertex",
            "surface_fragment",
            MTLPixelFormat::BGRA8Unorm,
            true,
        )?;
        // Blending is off so that the transparent output replaces what was drawn beneath it.
        let native_views_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "native_views",
            "native_view_vertex",
            "native_view_fragment",
            MTLPixelFormat::BGRA8Unorm,
            false,
        )?;
        // BenCode: clears a cutout to transparent black (`hole_punch_marker`).
        let clear_quads_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "clear_quads",
            "quad_vertex",
            "clear_quad_fragment",
            MTLPixelFormat::BGRA8Unorm,
            false,
        )?;
        let backdrop_composite_pipeline_state = build_backdrop_pipeline_state(
            &device,
            &library,
            "backdrop_composite",
            "backdrop_composite_vertex",
            "backdrop_composite_fragment",
            MTLPixelFormat::BGRA8Unorm,
        )?;

        let depth_write_state = build_depth_stencil_state(&device, true)?;
        let depth_test_state = build_depth_stencil_state(&device, false)?;

        let command_queue = device.newCommandQueue().context("creating command queue")?;
        let supports_shared_storage = cfg!(target_os = "ios") || is_apple_gpu;
        #[expect(
            clippy::arc_with_non_send_sync,
            reason = "PlatformWindow::sprite_atlas requires Arc, not Send or Sync"
        )]
        let sprite_atlas = Arc::new(MetalAtlas::new(device.clone(), supports_shared_storage));
        crate::metal_atlas::register(&sprite_atlas);
        let mut cache = ptr::null_mut();
        // SAFETY: The output pointer is writable and both optional attribute dictionaries are absent.
        let status = unsafe {
            CVMetalTextureCache::create(None, None, &device, None, ptr::NonNull::from(&mut cache))
        };
        anyhow::ensure!(
            status == kCVReturnSuccess,
            "creating CoreVideo Metal texture cache: status {status}"
        );
        let cache = ptr::NonNull::new(cache).context("retrieving CoreVideo Metal texture cache")?;
        // SAFETY: The successful create call transfers ownership of this non-null cache.
        let core_video_texture_cache = unsafe { CFRetained::from_raw(cache) };

        Ok(Self {
            device,
            layer,
            presents_with_transaction: false,
            display_link_mode: DisplayLinkMode::Detached,
            is_apple_gpu,
            is_unified_memory,
            opaque,
            shows_native_views: false,
            command_queue,
            path_tiles_pipeline_state,
            shadows_pipeline_state,
            quads_pipeline_state,
            opaque_quads_pipeline_state,
            underlines_pipeline_state,
            monochrome_sprites_pipeline_state,
            polychrome_sprites_pipeline_state,
            surfaces_pipeline_state,
            native_views_pipeline_state,
            clear_quads_pipeline_state,
            chat_background_pipeline_state: None,
            library,
            backdrop_composite_pipeline_state,
            backdrop_kernels: Vec::new(),
            backdrop_scratch: None,
            backdrop_used: None,
            trim_generation: TRIM_GENERATION.load(Ordering::Relaxed),
            depth_write_state,
            depth_test_state,
            depth_texture: None,
            instance_buffer_pool,
            sprite_atlas,
            core_video_texture_cache,
            #[cfg(any(test, feature = "bench-support", feature = "test-support"))]
            headless_render_target: None,
        })
    }

    pub fn layer(&self) -> Option<&CAMetalLayer> {
        self.layer.as_deref()
    }

    pub fn layer_ptr(&self) -> *mut CAMetalLayer {
        self.layer
            .as_ref()
            .map(|layer| Retained::as_ptr(layer).cast_mut())
            .unwrap_or(ptr::null_mut())
    }

    pub fn sprite_atlas(&self) -> &Arc<MetalAtlas> {
        &self.sprite_atlas
    }

    /// Draw the next frame into `drawable` instead of asking the layer for one.
    ///
    /// A `CAMetalDisplayLink` hands out each frame's drawable early enough for
    /// the frame to reach the display on time; `nextDrawable` would instead
    /// wait for the compositor to release one. Pass `None` once the frame is
    /// done, so a drawable the frame did not use goes back to the layer.
    ///
    /// Returns the drawable this replaces: after a frame, `Some` means the
    /// frame neither drew nor presented.
    ///
    /// This is a setter rather than a guard scoped to one frame because the
    /// window releases its lock on the renderer while GPUI draws.
    pub fn set_frame_drawable(
        &mut self,
        drawable: Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
    ) -> Option<Retained<ProtocolObject<dyn CAMetalDrawable>>> {
        match &mut self.display_link_mode {
            DisplayLinkMode::Attached { drawable: current } => mem::replace(current, drawable),
            DisplayLinkMode::Detached => {
                // Only a live link hands out drawables, and a link is detached
                // only after it has been invalidated.
                debug_assert!(
                    drawable.is_none(),
                    "received a display link drawable while detached"
                );
                None
            }
        }
    }

    /// Record whether a `CAMetalDisplayLink` drives the layer. While one
    /// does, frames are drawn only into the drawables it hands out.
    pub fn set_display_link_attached(&mut self, attached: bool) {
        self.display_link_mode = if attached {
            DisplayLinkMode::Attached { drawable: None }
        } else {
            DisplayLinkMode::Detached
        };
    }

    pub fn is_display_link_attached(&self) -> bool {
        matches!(self.display_link_mode, DisplayLinkMode::Attached { .. })
    }

    pub fn presents_with_transaction(&self) -> bool {
        self.presents_with_transaction
    }

    pub fn set_presents_with_transaction(&mut self, presents_with_transaction: bool) {
        self.presents_with_transaction = presents_with_transaction;
        if let Some(layer) = &self.layer {
            layer.setPresentsWithTransaction(presents_with_transaction);
        }
    }

    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        if let Some(layer) = &self.layer {
            layer.setDrawableSize(CGSize::new(size.width.0 as f64, size.height.0 as f64));
        }
    }

    pub fn update_transparency(&mut self, transparent: bool) {
        self.opaque = !transparent;
        if let Some(layer) = &self.layer {
            layer.setOpaque(self.opaque && !self.shows_native_views);
        }
    }

    pub fn destroy(&self) {
        // nothing to do
    }

    pub fn draw(&mut self, scene: &Scene) {
        let layer = match &self.layer {
            Some(l) => l.clone(),
            None => {
                log::error!(
                    "draw() called on headless renderer - use render_scene_to_image() instead"
                );
                return;
            }
        };
        // Native views sit beneath the layer, so it can only be opaque, and
        // eligible for direct-to-display presentation, while none are showing.
        // BenCode's cutouts show them too (`hole_punch_marker`).
        let shows_native_views =
            !scene.native_views.is_empty() || scene.quads.iter().any(is_hole_punch);
        if shows_native_views != self.shows_native_views {
            self.shows_native_views = shows_native_views;
            layer.setOpaque(self.opaque && !shows_native_views);
        }
        let drawable = match &mut self.display_link_mode {
            DisplayLinkMode::Attached { drawable } => {
                let Some(drawable) = drawable.take() else {
                    log::error!("skipped a frame drawn outside its display link's update");
                    return;
                };
                drawable
            }
            DisplayLinkMode::Detached => {
                let Some(drawable) = layer.nextDrawable() else {
                    log::error!(
                        "failed to retrieve next drawable, drawable size: {:?}",
                        layer.drawableSize()
                    );
                    return;
                };
                drawable
            }
        };
        // A display link's drawable was sized when the link handed it out,
        // which can be before a resize the layer has since taken.
        let texture = drawable.texture();
        let viewport_size: Size<DevicePixels> = size(
            (texture.width() as i32).into(),
            (texture.height() as i32).into(),
        );

        let command_buffer = match self.render_frame(scene, &drawable.texture(), viewport_size) {
            Ok(command_buffer) => command_buffer,
            Err(error) => {
                log::error!("failed to render: {error:#}");
                return;
            }
        };

        if self.presents_with_transaction {
            command_buffer.commit();
            command_buffer.waitUntilScheduled();
            drawable.present();
        } else {
            command_buffer.presentDrawable(ProtocolObject::from_ref(&*drawable));
            command_buffer.commit();
        }
    }

    fn render_frame(
        &mut self,
        scene: &Scene,
        texture: &ProtocolObject<dyn MTLTexture>,
        viewport_size: Size<DevicePixels>,
    ) -> Result<Retained<ProtocolObject<dyn MTLCommandBuffer>>> {
        let mut writer = InstanceBufferWriter::new(
            &self.device,
            &self.instance_buffer_pool,
            self.is_unified_memory,
        )?;
        let instance_bindings = write_instances(scene, &mut writer).with_context(|| {
            format!(
                "writing instance data: {} paths, {} shadows, {} quads, {} underlines, {} mono, {} poly, {} surfaces",
                scene.paths.len(),
                scene.shadows.len(),
                scene.quads.len(),
                scene.underlines.len(),
                scene.monochrome_sprites.len(),
                scene.polychrome_sprites.len(),
                scene.surfaces.len(),
            )
        })?;
        let mut surface_textures = Vec::new();
        let command_buffer = self.draw_primitives_to_texture(
            scene,
            &instance_bindings,
            texture,
            viewport_size,
            &mut surface_textures,
        )?;
        self.release_idle_memory();

        let instance_buffer_pool = self.instance_buffer_pool.clone();
        let frame_resources = Cell::new(Some((writer.finish(), surface_textures)));
        let block = RcBlock::new(
            move |_: ptr::NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                if let Some((instance_buffer, _surface_textures)) = frame_resources.take() {
                    instance_buffer_pool.lock().release(instance_buffer);
                }
            },
        );
        // SAFETY: Metal copies the live block and invokes it once after this frame completes.
        // Its captures own the frame resources and use the pool mutex when returning the buffer.
        unsafe {
            command_buffer.addCompletedHandler(RcBlock::as_ptr(&block));
        }

        Ok(command_buffer)
    }

    /// Renders the scene to a texture and returns the pixel data as an RGBA image.
    /// This does not present the frame to screen - useful for visual testing
    /// where we want to capture what would be rendered without displaying it.
    ///
    /// Note: This requires a layer-backed renderer. For headless rendering,
    /// use `render_scene_to_image()` instead.
    #[cfg(any(test, feature = "test-support"))]
    pub fn render_to_image(&mut self, scene: &Scene) -> Result<RgbaImage> {
        let layer = self
            .layer
            .clone()
            .ok_or_else(|| anyhow::anyhow!("render_to_image requires a layer-backed renderer"))?;
        let viewport_size = layer.drawableSize();
        let viewport_size: Size<DevicePixels> = size(
            (viewport_size.width.ceil() as i32).into(),
            (viewport_size.height.ceil() as i32).into(),
        );
        // CAMetalDisplayLink owns the drawable, so render into a texture we own.
        self.render_offscreen(scene, viewport_size)
    }

    /// Renders a scene to an image without requiring a window or CAMetalLayer.
    ///
    /// This is the primary method for headless rendering. It creates an offscreen
    /// texture, renders the scene to it, and returns the pixel data as an RGBA image.
    #[cfg(any(test, feature = "bench-support", feature = "test-support"))]
    pub fn render_scene_to_image(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> Result<RgbaImage> {
        if size.width.0 <= 0 || size.height.0 <= 0 {
            anyhow::bail!("Invalid size for render_scene_to_image: {:?}", size);
        }

        // Headless callers do not have a Cocoa event-loop pool to release
        // autoreleased command buffers and render-pass descriptors.
        objc2::rc::autoreleasepool(|_| self.render_offscreen(scene, size))
    }

    /// Renders a scene into a new offscreen texture of `size` and reads it
    /// back, without presenting anything.
    #[cfg(any(test, feature = "bench-support", feature = "test-support"))]
    fn render_offscreen(&mut self, scene: &Scene, size: Size<DevicePixels>) -> Result<RgbaImage> {
        if size.width.0 <= 0 || size.height.0 <= 0 {
            anyhow::bail!("Invalid size for render_offscreen: {:?}", size);
        }

        let texture_descriptor = MTLTextureDescriptor::new();
        // SAFETY: The dimensions are positive and this descriptor creates a 2D texture.
        unsafe {
            texture_descriptor.setWidth(size.width.0 as usize);
            texture_descriptor.setHeight(size.height.0 as usize);
        }
        texture_descriptor.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        texture_descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        // Like the atlas, only Apple GPUs can create shared textures on macOS;
        // Intel Macs cannot, even with unified memory. iOS has no managed storage.
        let uses_shared_storage = cfg!(target_os = "ios") || self.is_apple_gpu;
        texture_descriptor.setStorageMode(if uses_shared_storage {
            MTLStorageMode::Shared
        } else {
            MTLStorageMode::Managed
        });
        let target_texture = self
            .device
            .newTextureWithDescriptor(&texture_descriptor)
            .context("allocating offscreen render target")?;

        let command_buffer = self.render_frame(scene, &target_texture, size)?;

        // Managed textures require an explicit blit synchronize before the CPU
        // can read back the rendered data. Without this, get_bytes returns
        // stale zeros.
        if !uses_shared_storage {
            let blit = command_buffer
                .blitCommandEncoder()
                .context("creating blit command encoder")?;
            blit.synchronizeResource(ProtocolObject::from_ref(&*target_texture));
            blit.endEncoding();
        }

        command_buffer.commit();
        command_buffer.waitUntilCompleted();

        read_texture_to_image(&target_texture)
    }

    /// Renders a scene to a reused offscreen texture without reading pixels
    /// back or blocking on GPU completion.
    ///
    /// This mirrors the CPU cost of presenting a frame to a window (scene
    /// encoding, instance buffer writes, command submission) and is used by
    /// headless benchmark rendering, where the produced pixels are never
    /// inspected.
    #[cfg(any(test, feature = "bench-support", feature = "test-support"))]
    pub fn render_scene(&mut self, scene: &Scene, size: Size<DevicePixels>) -> Result<()> {
        if size.width.0 <= 0 || size.height.0 <= 0 {
            anyhow::bail!("Invalid size for render_scene: {:?}", size);
        }

        objc2::rc::autoreleasepool(|_| {
            let needs_new_target = self.headless_render_target.as_ref().is_none_or(|texture| {
                texture.width() != size.width.0 as usize
                    || texture.height() != size.height.0 as usize
            });
            if needs_new_target {
                let texture_descriptor = MTLTextureDescriptor::new();
                unsafe {
                    texture_descriptor.setWidth(size.width.0 as usize);
                    texture_descriptor.setHeight(size.height.0 as usize);
                }
                texture_descriptor.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
                texture_descriptor
                    .setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
                texture_descriptor.setStorageMode(MTLStorageMode::Private);
                self.headless_render_target = Some(
                    self.device
                        .newTextureWithDescriptor(&texture_descriptor)
                        .context("allocating headless render target")?,
                );
            }
            let target_texture = self
                .headless_render_target
                .clone()
                .context("retrieving headless render target")?;

            let command_buffer = self.render_frame(scene, &target_texture, size)?;

            // Commit without waiting, mirroring presentation to a real window where
            // the CPU doesn't block on the GPU.
            command_buffer.commit();
            Ok(())
        })
    }

    fn draw_primitives_to_texture(
        &mut self,
        scene: &Scene,
        instance_bindings: &InstanceBindings,
        texture: &ProtocolObject<dyn MTLTexture>,
        viewport_size: Size<DevicePixels>,
        surface_textures: &mut Vec<CFRetained<CVMetalTexture>>,
    ) -> Result<Retained<ProtocolObject<dyn MTLCommandBuffer>>> {
        let command_queue = self.command_queue.clone();
        let command_buffer = command_queue
            .commandBuffer()
            .context("creating command buffer")?;
        let alpha = if self.opaque { 1. } else { 0. };
        let depth_texture = self.depth_texture_matching(texture)?;

        let mut command_encoder = new_command_encoder_for_texture(
            &command_buffer,
            texture,
            &depth_texture,
            viewport_size,
            Some(MTLClearColor {
                red: 0.,
                green: 0.,
                blue: 0.,
                alpha,
            }),
        )?;
        let depth_scale = quad_depth_scale(scene.quads.len());
        // BenCode: a frosted quad copies what is drawn before it, and a cutout
        // clears it, so the opaque pass (which draws ahead of everything else)
        // stops at the first of them; the quad pass draws the opaque quads
        // after it in order.
        let opaque_quads = scene
            .quads
            .iter()
            .position(|quad| is_hole_punch(quad) || backdrop_sigma(quad).is_some())
            .unwrap_or(scene.quads.len());
        bind_frame_inputs(&command_encoder, viewport_size, depth_scale, opaque_quads);

        self.draw_opaque_quads(opaque_quads, instance_bindings, &command_encoder);
        command_encoder.setDepthStencilState(Some(&self.depth_test_state));

        let hole_bounds: Vec<Bounds<ScaledPixels>> = scene
            .quads
            .iter()
            .filter(|quad| is_hole_punch(quad))
            .map(|quad| quad.bounds.intersect(&quad.content_mask.bounds))
            .filter(|bounds| bounds.size.width.0 > 0. && bounds.size.height.0 > 0.)
            .collect();

        let mut quad_cursor: u32 = 0;
        let mut path_tile_cursor = 0;
        for batch in scene.batches() {
            // Quad shaders assign each quad its own depth.
            // In every other batch type, the depth is fixed to the position of the quad cursor.
            let (znear, zfar) = if matches!(&batch, PrimitiveBatch::Quads(_)) {
                (0.0, 1.0)
            } else {
                let depth = quad_depth(quad_cursor, depth_scale) as f64;
                (depth, depth)
            };
            command_encoder.setViewport(MTLViewport {
                originX: 0.0,
                originY: 0.0,
                width: i32::from(viewport_size.width) as f64,
                height: i32::from(viewport_size.height) as f64,
                znear,
                zfar,
            });

            match batch {
                PrimitiveBatch::Shadows(range) => {
                    self.draw_shadows(range, instance_bindings, &command_encoder)
                }
                PrimitiveBatch::Quads(range) => {
                    quad_cursor += range.len() as u32;
                    // A marked quad either clears a transparent cutout for a
                    // native view behind GPUI (`hole_punch_marker`) or frosts
                    // what is already drawn behind it (`backdrop_blur_marker`).
                    let mut start = range.start;
                    let mut ix = range.start;
                    while ix < range.end {
                        let quad = &scene.quads[ix];
                        if is_hole_punch(quad) {
                            self.draw_quads(start..ix, instance_bindings, &command_encoder);
                            self.draw_clear_quad(ix, instance_bindings, &command_encoder);
                            ix += 1;
                            start = ix;
                            continue;
                        }
                        let Some(sigma) = backdrop_sigma(quad) else {
                            ix += 1;
                            continue;
                        };
                        // GPUI draws a rounded quad as pieces (its interior
                        // and four strips, each the whole quad under its own
                        // mask). Pieces drawn one after another see the same
                        // backdrop, so they share one blur.
                        let pieces = ix
                            ..(ix + 1..range.end)
                                .find(|&next| !same_frost(&scene.quads[next], quad))
                                .unwrap_or(range.end);
                        self.draw_quads(start..ix, instance_bindings, &command_encoder);
                        command_encoder.endEncoding();
                        let pieces = &scene.quads[pieces];
                        let overlaps_hole = pieces.iter().any(|piece| {
                            let clipped = piece.bounds.intersect(&piece.content_mask.bounds);
                            hole_bounds.iter().any(|h| h.intersects(&clipped))
                        });
                        let fallback_alpha = if overlaps_hole { 0.88 } else { 0. };
                        self.blur_backdrop(
                            pieces,
                            sigma,
                            fallback_alpha,
                            texture,
                            viewport_size,
                            &command_buffer,
                        )?;
                        // Everything left is in front of the opaque pass, so
                        // the new pass starts its depth afresh.
                        command_encoder = new_command_encoder_for_texture(
                            &command_buffer,
                            texture,
                            &depth_texture,
                            viewport_size,
                            None,
                        )?;
                        bind_frame_inputs(
                            &command_encoder,
                            viewport_size,
                            depth_scale,
                            opaque_quads,
                        );
                        command_encoder.setDepthStencilState(Some(&self.depth_test_state));
                        // The pieces themselves are drawn next, as the tint.
                        start = ix;
                        ix += pieces.len();
                    }
                    self.draw_quads(start..range.end, instance_bindings, &command_encoder)
                }
                PrimitiveBatch::Paths(range) => {
                    let tiles_start = path_tile_cursor;
                    path_tile_cursor += scene.paths[range]
                        .iter()
                        .map(PaintedPath::tile_count)
                        .sum::<usize>();
                    self.draw_path_tiles(
                        tiles_start..path_tile_cursor,
                        instance_bindings,
                        &command_encoder,
                    )
                }
                PrimitiveBatch::Underlines(range) => {
                    self.draw_underlines(range, instance_bindings, &command_encoder)
                }
                PrimitiveBatch::MonochromeSprites { texture_id, range } => self
                    .draw_monochrome_sprites(
                        texture_id,
                        range,
                        instance_bindings,
                        &command_encoder,
                    ),
                PrimitiveBatch::PolychromeSprites { texture_id, range } => self
                    .draw_polychrome_sprites(
                        texture_id,
                        &scene.polychrome_sprites[range.clone()],
                        range.start,
                        instance_bindings,
                        &command_encoder,
                    ),
                PrimitiveBatch::Surfaces(range) => {
                    if let Err(error) = self.draw_surfaces(
                        &scene.surfaces[range.clone()],
                        range.start,
                        instance_bindings,
                        &command_encoder,
                        surface_textures,
                    ) {
                        command_encoder.endEncoding();
                        return Err(error);
                    }
                }
                PrimitiveBatch::NativeViews(range) => {
                    self.draw_native_views(range, instance_bindings, &command_encoder)
                }
                PrimitiveBatch::SubpixelSprites { .. } => unreachable!(),
            }
        }

        command_encoder.endEncoding();

        Ok(command_buffer)
    }

    /// Drops the backdrop's textures once no recent frame frosted anything,
    /// and after a [`trim`] the instance buffers grown past their first size
    /// and the depth buffer. Encoded frames keep their own references until
    /// the GPU finishes.
    fn release_idle_memory(&mut self) {
        if self
            .backdrop_used
            .is_none_or(|used| used.elapsed() >= SCRATCH_IDLE)
        {
            self.backdrop_scratch = None;
            self.backdrop_used = None;
        }
        let generation = TRIM_GENERATION.load(Ordering::Relaxed);
        if self.trim_generation != generation {
            self.trim_generation = generation;
            self.depth_texture = None;
            let mut pool = self.instance_buffer_pool.lock();
            if pool.buffer_size != DEFAULT_INSTANCE_BUFFER_SIZE || !pool.buffers.is_empty() {
                log::info!("trimmed the instance buffers back to {DEFAULT_INSTANCE_BUFFER_SIZE}");
                pool.reset(DEFAULT_INSTANCE_BUFFER_SIZE);
            }
        }
    }

    /// Replaces the pixels inside the frosted quad's rounded bounds with a
    /// Gaussian blur (`sigma` device pixels) of what is already drawn there
    /// and around it, like CSS `backdrop-filter: blur()`. The quad itself is
    /// drawn afterwards.
    ///
    /// `pieces` are the parts GPUI split one quad into (`same_frost`): each
    /// writes only inside its own mask, but its edge is the whole quad's, so
    /// the pieces meet without a seam.
    ///
    /// The blur uses Apple's `MPSImageGaussianBlur` at
    /// full resolution with clamped edges: downsampled or sparse-tap blurs
    /// ghost text that sits under the glass.
    fn blur_backdrop(
        &mut self,
        pieces: &[Quad],
        sigma: f32,
        fallback_alpha: f32,
        target: &ProtocolObject<dyn MTLTexture>,
        viewport_size: Size<DevicePixels>,
        command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
    ) -> Result<()> {
        let viewport = [
            i32::from(viewport_size.width) as f32,
            i32::from(viewport_size.height) as f32,
        ];
        let Some(quad) = pieces.first() else {
            return Ok(());
        };
        // Frosting stops where the quad is clipped, like the quad itself: a
        // pinned header pushed under a toolbar must not blur the toolbar.
        let clipped: Vec<Bounds<ScaledPixels>> = pieces
            .iter()
            .map(|piece| piece.bounds.intersect(&piece.content_mask.bounds))
            .filter(|bounds| bounds.size.width.0 > 0. && bounds.size.height.0 > 0.)
            .collect();
        let Some(bounds) = clipped.iter().copied().reduce(|a, b| a.union(&b)) else {
            return Ok(());
        };
        // The interior piece has its corners squared off; a strip keeps them.
        let corner_radius = pieces
            .iter()
            .map(|piece| piece.corner_radii.top_left.0)
            .fold(0., f32::max);
        // The gaussian reaches about 3 sigma, so snapshot that much around
        // the glass: the blur near its edges then averages the backdrop
        // beyond them, as the stylesheet's bleed does, instead of fading.
        let bleed = (sigma * 3.).ceil() + 2.;
        let left = (bounds.origin.x.0 - bleed).floor().max(0.);
        let top = (bounds.origin.y.0 - bleed).floor().max(0.);
        let right = (bounds.origin.x.0 + bounds.size.width.0 + bleed)
            .ceil()
            .min(viewport[0]);
        let bottom = (bounds.origin.y.0 + bounds.size.height.0 + bleed)
            .ceil()
            .min(viewport[1]);
        if right - left < 1. || bottom - top < 1. || sigma <= 0. {
            return Ok(());
        }
        let region = ((right - left) as usize, (bottom - top) as usize);
        let format = target.pixelFormat();
        let reusable = self.backdrop_scratch.as_ref().is_some_and(|(original, _)| {
            original.width() == region.0
                && original.height() == region.1
                && original.pixelFormat() == format
        });
        if !reusable {
            let descriptor = MTLTextureDescriptor::new();
            // SAFETY: The region is at least one pixel and within the drawable.
            unsafe {
                descriptor.setWidth(region.0);
                descriptor.setHeight(region.1);
            }
            descriptor.setPixelFormat(format);
            descriptor.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::ShaderWrite);
            descriptor.setStorageMode(MTLStorageMode::Private);
            let new_texture = || {
                self.device
                    .newTextureWithDescriptor(&descriptor)
                    .context("allocating backdrop texture")
            };
            self.backdrop_scratch = Some((new_texture()?, new_texture()?));
        }
        self.backdrop_used = Some(Instant::now());
        let kernel = self.gaussian_kernel(sigma)?;
        let (original, blurred) = self
            .backdrop_scratch
            .as_ref()
            .context("retrieving backdrop textures")?;

        let blit = command_buffer
            .blitCommandEncoder()
            .context("creating backdrop blit encoder")?;
        // SAFETY: The region lies within `target` and fits `original`, which
        // share a pixel format.
        unsafe {
            blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                target,
                0,
                0,
                MTLOrigin {
                    x: left as usize,
                    y: top as usize,
                    z: 0,
                },
                MTLSize {
                    width: region.0,
                    height: region.1,
                    depth: 1,
                },
                original,
                0,
                0,
                MTLOrigin { x: 0, y: 0, z: 0 },
            );
        }
        blit.endEncoding();
        // SAFETY: all three are live Metal objects owned for this frame, and
        // the selector matches MPSUnaryImageKernel's encoding method.
        unsafe {
            let _: () = msg_send![
                &*kernel,
                encodeToCommandBuffer: (command_buffer as *const ProtocolObject<dyn MTLCommandBuffer>)
                    .cast::<AnyObject>(),
                sourceTexture: Retained::as_ptr(original).cast::<AnyObject>(),
                destinationTexture: Retained::as_ptr(blurred).cast::<AnyObject>()
            ];
        }

        let render_pass_descriptor = MTLRenderPassDescriptor::new();
        // SAFETY: Index zero is within the render pass's color attachment array.
        let color_attachment = unsafe {
            render_pass_descriptor
                .colorAttachments()
                .objectAtIndexedSubscript(0)
        };
        color_attachment.setTexture(Some(target));
        color_attachment.setLoadAction(MTLLoadAction::Load);
        color_attachment.setStoreAction(MTLStoreAction::Store);
        let encoder = command_buffer
            .renderCommandEncoderWithDescriptor(&render_pass_descriptor)
            .context("creating backdrop composite encoder")?;
        encoder.setRenderPipelineState(&self.backdrop_composite_pipeline_state);
        // SAFETY: the textures live for the frame.
        unsafe {
            encoder.setFragmentTexture_atIndex(Some(blurred), 0);
            encoder.setFragmentTexture_atIndex(Some(original), 1);
        }
        for piece in &clipped {
            let params = BackdropCompositeParams {
                origin: [piece.origin.x.0, piece.origin.y.0],
                size: [piece.size.width.0, piece.size.height.0],
                shape_origin: [quad.bounds.origin.x.0, quad.bounds.origin.y.0],
                shape_size: [quad.bounds.size.width.0, quad.bounds.size.height.0],
                region_origin: [left, top],
                region_size: [region.0 as f32, region.1 as f32],
                viewport_size: viewport,
                corner_radius,
                fallback_alpha,
            };
            // SAFETY: Metal copies the parameters, which match the composite
            // shaders' `BackdropCompositeParams`.
            unsafe {
                let params_ptr = ptr::NonNull::from(&params).cast();
                let params_len = mem::size_of_val(&params);
                encoder.setVertexBytes_length_atIndex(params_ptr, params_len, 0);
                encoder.setFragmentBytes_length_atIndex(params_ptr, params_len, 0);
                encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 6);
            }
        }
        encoder.endEncoding();
        Ok(())
    }

    /// The cached `MPSImageGaussianBlur` for `sigma` device pixels.
    fn gaussian_kernel(&mut self, sigma: f32) -> Result<Retained<AnyObject>> {
        if let Some(ix) = self
            .backdrop_kernels
            .iter()
            .position(|(cached, _)| (cached - sigma).abs() < 0.01)
        {
            let entry = self.backdrop_kernels.remove(ix);
            let kernel = entry.1.clone();
            self.backdrop_kernels.push(entry);
            return Ok(kernel);
        }
        if self.backdrop_kernels.len() >= 4 {
            self.backdrop_kernels.remove(0);
        }
        let class = AnyClass::get(c"MPSImageGaussianBlur")
            .context("MetalPerformanceShaders lacks MPSImageGaussianBlur")?;
        // SAFETY: alloc/init returns a +1 reference that Retained takes over;
        // the device pointer is this renderer's live MTLDevice.
        let kernel = unsafe {
            let allocated: *mut AnyObject = msg_send![class, alloc];
            let kernel: *mut AnyObject = msg_send![
                allocated,
                initWithDevice: Retained::as_ptr(&self.device).cast::<AnyObject>(),
                sigma: sigma
            ];
            let kernel = Retained::from_raw(kernel).context("MPSImageGaussianBlur init failed")?;
            // MPSImageEdgeModeClamp: the default zero edge bleeds transparent
            // black into blurs near the window border.
            let _: () = msg_send![&*kernel, setEdgeMode: 1usize];
            kernel
        };
        self.backdrop_kernels.push((sigma, kernel.clone()));
        Ok(kernel)
    }

    fn depth_texture_matching(
        &mut self,
        target: &ProtocolObject<dyn MTLTexture>,
    ) -> Result<Retained<ProtocolObject<dyn MTLTexture>>> {
        if let Some(depth_texture) = &self.depth_texture
            && depth_texture.width() == target.width()
            && depth_texture.height() == target.height()
        {
            return Ok(depth_texture.clone());
        }
        let descriptor = MTLTextureDescriptor::new();
        // SAFETY: The size is the size of an existing render target, which the device supports.
        unsafe {
            descriptor.setWidth(target.width());
            descriptor.setHeight(target.height());
        }
        descriptor.setPixelFormat(MTLPixelFormat::Depth32Float);
        descriptor.setUsage(MTLTextureUsage::RenderTarget);
        // The depth buffer never leaves its render pass, so Apple GPUs can keep
        // it in tile memory without allocating any backing store.
        descriptor.setStorageMode(if self.is_apple_gpu {
            MTLStorageMode::Memoryless
        } else {
            MTLStorageMode::Private
        });
        let depth_texture = self
            .device
            .newTextureWithDescriptor(&descriptor)
            .context("allocating depth texture")?;
        self.depth_texture = Some(depth_texture.clone());
        Ok(depth_texture)
    }

    fn draw_opaque_quads(
        &self,
        quad_count: usize,
        instance_bindings: &InstanceBindings,
        command_encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    ) {
        if quad_count == 0 {
            return;
        }

        command_encoder.setRenderPipelineState(&self.opaque_quads_pipeline_state);
        command_encoder.setDepthStencilState(Some(&self.depth_write_state));
        // SAFETY: The scene's quads match the shader layout and slot, and
        // `bind_frame_inputs` bound `quad_count` as the shader's count.
        // Metal retains the buffer, and writer.finish publishes its writes.
        unsafe {
            command_encoder.setVertexBuffer_offset_atIndex(
                Some(&instance_bindings.quads.buffer),
                instance_bindings.quads.offset,
                QuadInputIndex::Quads as usize,
            );
            command_encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                MTLPrimitiveType::TriangleStrip,
                0,
                4,
                quad_count,
                0,
            );
        }
    }

    /// Clears quad `quad_index`'s clipped bounds to transparent black, a
    /// cutout for a native view behind GPUI (`hole_punch_marker`).
    fn draw_clear_quad(
        &self,
        quad_index: usize,
        instance_bindings: &InstanceBindings,
        command_encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    ) {
        command_encoder.setRenderPipelineState(&self.clear_quads_pipeline_state);
        // SAFETY: The quad index lies within the scene's uploaded quads.
        unsafe {
            command_encoder.setVertexBuffer_offset_atIndex(
                Some(&instance_bindings.quads.buffer),
                instance_bindings.quads.offset,
                QuadInputIndex::Quads as usize,
            );
            command_encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                MTLPrimitiveType::TriangleStrip,
                0,
                4,
                1,
                quad_index,
            );
        }
    }

    fn draw_native_views(
        &self,
        native_views: Range<usize>,
        instance_bindings: &InstanceBindings,
        command_encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    ) {
        if native_views.is_empty() {
            return;
        }

        command_encoder.setRenderPipelineState(&self.native_views_pipeline_state);
        // SAFETY: The SurfaceBounds instances match the shader layout and slot.
        // Metal retains the buffer, and writer.finish publishes its writes.
        unsafe {
            command_encoder.setVertexBuffer_offset_atIndex(
                Some(&instance_bindings.native_views.buffer),
                instance_bindings.native_views.offset,
                SurfaceInputIndex::Surfaces as usize,
            );
            command_encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                MTLPrimitiveType::TriangleStrip,
                0,
                4,
                native_views.len(),
                native_views.start,
            );
        }
    }

    fn draw_path_tiles(
        &self,
        tiles: Range<usize>,
        instance_bindings: &InstanceBindings,
        command_encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    ) {
        if tiles.is_empty() {
            return;
        }

        command_encoder.setRenderPipelineState(&self.path_tiles_pipeline_state);
        // SAFETY: The scene's tile range and its curves and paints match the uploaded shader layouts and slots.
        // Metal retains the buffers, and writer.finish publishes their writes.
        unsafe {
            command_encoder.setVertexBuffer_offset_atIndex(
                Some(&instance_bindings.path_tiles.buffer),
                instance_bindings.path_tiles.offset,
                PathTileInputIndex::Tiles as usize,
            );
            command_encoder.setVertexBuffer_offset_atIndex(
                Some(&instance_bindings.path_paints.buffer),
                instance_bindings.path_paints.offset,
                PathTileInputIndex::Paints as usize,
            );
            command_encoder.setFragmentBuffer_offset_atIndex(
                Some(&instance_bindings.path_tiles.buffer),
                instance_bindings.path_tiles.offset,
                PathTileInputIndex::Tiles as usize,
            );
            command_encoder.setFragmentBuffer_offset_atIndex(
                Some(&instance_bindings.path_curves.buffer),
                instance_bindings.path_curves.offset,
                PathTileInputIndex::Curves as usize,
            );
            command_encoder.setFragmentBuffer_offset_atIndex(
                Some(&instance_bindings.path_paints.buffer),
                instance_bindings.path_paints.offset,
                PathTileInputIndex::Paints as usize,
            );
            command_encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                MTLPrimitiveType::TriangleStrip,
                0,
                4,
                tiles.len(),
                tiles.start,
            );
        }
    }

    fn draw_shadows(
        &self,
        shadows: Range<usize>,
        instance_bindings: &InstanceBindings,
        command_encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    ) {
        if shadows.is_empty() {
            return;
        }

        command_encoder.setRenderPipelineState(&self.shadows_pipeline_state);
        // SAFETY: The scene's shadow range matches the uploaded shader layouts and slots.
        // Metal retains the buffers, and writer.finish publishes their writes.
        unsafe {
            command_encoder.setVertexBuffer_offset_atIndex(
                Some(&instance_bindings.shadows.buffer),
                instance_bindings.shadows.offset,
                ShadowInputIndex::Shadows as usize,
            );
            command_encoder.setFragmentBuffer_offset_atIndex(
                Some(&instance_bindings.shadows.buffer),
                instance_bindings.shadows.offset,
                ShadowInputIndex::Shadows as usize,
            );
            command_encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                MTLPrimitiveType::TriangleStrip,
                0,
                4,
                shadows.len(),
                shadows.start,
            );
        }
    }

    fn draw_quads(
        &self,
        quads: Range<usize>,
        instance_bindings: &InstanceBindings,
        command_encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    ) {
        if quads.is_empty() {
            return;
        }

        command_encoder.setRenderPipelineState(&self.quads_pipeline_state);
        // SAFETY: The scene's quad range matches the uploaded shader layouts and slots.
        // Metal retains the buffers, and writer.finish publishes their writes.
        unsafe {
            command_encoder.setVertexBuffer_offset_atIndex(
                Some(&instance_bindings.quads.buffer),
                instance_bindings.quads.offset,
                QuadInputIndex::Quads as usize,
            );
            command_encoder.setFragmentBuffer_offset_atIndex(
                Some(&instance_bindings.quads.buffer),
                instance_bindings.quads.offset,
                QuadInputIndex::Quads as usize,
            );
            command_encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                MTLPrimitiveType::TriangleStrip,
                0,
                4,
                quads.len(),
                quads.start,
            );
        }
    }

    fn draw_underlines(
        &self,
        underlines: Range<usize>,
        instance_bindings: &InstanceBindings,
        command_encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    ) {
        if underlines.is_empty() {
            return;
        }

        command_encoder.setRenderPipelineState(&self.underlines_pipeline_state);
        // SAFETY: The scene's underline range matches the uploaded shader layouts and slots.
        // Metal retains the buffers, and writer.finish publishes their writes.
        unsafe {
            command_encoder.setVertexBuffer_offset_atIndex(
                Some(&instance_bindings.underlines.buffer),
                instance_bindings.underlines.offset,
                UnderlineInputIndex::Underlines as usize,
            );
            command_encoder.setFragmentBuffer_offset_atIndex(
                Some(&instance_bindings.underlines.buffer),
                instance_bindings.underlines.offset,
                UnderlineInputIndex::Underlines as usize,
            );
            command_encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                MTLPrimitiveType::TriangleStrip,
                0,
                4,
                underlines.len(),
                underlines.start,
            );
        }
    }

    fn draw_monochrome_sprites(
        &self,
        texture_id: AtlasTextureId,
        sprites: Range<usize>,
        instance_bindings: &InstanceBindings,
        command_encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    ) {
        if sprites.is_empty() {
            return;
        }

        let Some(texture) = self.sprite_atlas.metal_texture(texture_id) else {
            return;
        };
        let texture_size = size(
            DevicePixels(texture.width() as i32),
            DevicePixels(texture.height() as i32),
        );
        command_encoder.setRenderPipelineState(&self.monochrome_sprites_pipeline_state);
        // SAFETY: The scene's sprite range and copied sizes match the shader layouts and slots.
        // Metal retains the buffers and texture, and writer.finish publishes the instance writes.
        unsafe {
            command_encoder.setVertexBuffer_offset_atIndex(
                Some(&instance_bindings.monochrome_sprites.buffer),
                instance_bindings.monochrome_sprites.offset,
                SpriteInputIndex::Sprites as usize,
            );
            command_encoder.setVertexBytes_length_atIndex(
                ptr::NonNull::from(&texture_size).cast(),
                mem::size_of_val(&texture_size),
                SpriteInputIndex::AtlasTextureSize as usize,
            );
            command_encoder.setFragmentBuffer_offset_atIndex(
                Some(&instance_bindings.monochrome_sprites.buffer),
                instance_bindings.monochrome_sprites.offset,
                SpriteInputIndex::Sprites as usize,
            );
            command_encoder.setFragmentTexture_atIndex(
                Some(&texture),
                SpriteInputIndex::AtlasTexture as usize,
            );
            command_encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                MTLPrimitiveType::TriangleStrip,
                0,
                4,
                sprites.len(),
                sprites.start,
            );
        }
    }

    fn draw_polychrome_sprites(
        &mut self,
        texture_id: AtlasTextureId,
        batch: &[PolychromeSprite],
        first_sprite: usize,
        instance_bindings: &InstanceBindings,
        command_encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    ) {
        if batch.is_empty() {
            return;
        }

        let Some(texture) = self.sprite_atlas.metal_texture(texture_id) else {
            return;
        };
        let texture_size = size(
            DevicePixels(texture.width() as i32),
            DevicePixels(texture.height() as i32),
        );
        // SAFETY: The scene's sprite range and copied sizes match the shader layouts and slots.
        // Metal retains the buffers and texture, and writer.finish publishes the instance writes.
        unsafe {
            command_encoder.setVertexBuffer_offset_atIndex(
                Some(&instance_bindings.polychrome_sprites.buffer),
                instance_bindings.polychrome_sprites.offset,
                SpriteInputIndex::Sprites as usize,
            );
            command_encoder.setVertexBytes_length_atIndex(
                ptr::NonNull::from(&texture_size).cast(),
                mem::size_of_val(&texture_size),
                SpriteInputIndex::AtlasTextureSize as usize,
            );
            command_encoder.setFragmentBuffer_offset_atIndex(
                Some(&instance_bindings.polychrome_sprites.buffer),
                instance_bindings.polychrome_sprites.offset,
                SpriteInputIndex::Sprites as usize,
            );
            command_encoder.setFragmentTexture_atIndex(
                Some(&texture),
                SpriteInputIndex::AtlasTexture as usize,
            );
        }

        // BenCode: a chat wallpaper (`chat_background_corners`) draws with its
        // own shader; the ordinary image shader stays as fast as it was.
        let is_background = |sprite: &PolychromeSprite| sprite.corner_radii.top_left.0 < 0.0;
        let mut start = first_sprite;
        for group in batch.chunk_by(|a, b| is_background(a) == is_background(b)) {
            let pipeline = if is_background(&group[0]) {
                if self.chat_background_pipeline_state.is_none() {
                    match build_pipeline_state(
                        &self.device,
                        &self.library,
                        "chat_background",
                        "polychrome_sprite_vertex",
                        "chat_background_fragment",
                        MTLPixelFormat::BGRA8Unorm,
                        true,
                    ) {
                        Ok(pipeline) => self.chat_background_pipeline_state = Some(pipeline),
                        Err(error) => log::error!("{error:#}"),
                    }
                }
                self.chat_background_pipeline_state
                    .as_ref()
                    .unwrap_or(&self.polychrome_sprites_pipeline_state)
            } else {
                &self.polychrome_sprites_pipeline_state
            };
            command_encoder.setRenderPipelineState(pipeline);
            // SAFETY: The group lies within this batch, whose sprites were bound above.
            unsafe {
                command_encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                    MTLPrimitiveType::TriangleStrip,
                    0,
                    4,
                    group.len(),
                    start,
                );
            }
            start += group.len();
        }
    }

    fn draw_surfaces(
        &mut self,
        surfaces: &[PaintSurface],
        first_surface: usize,
        instance_bindings: &InstanceBindings,
        command_encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
        surface_textures: &mut Vec<CFRetained<CVMetalTexture>>,
    ) -> Result<()> {
        if surfaces.is_empty() {
            return Ok(());
        }

        command_encoder.setRenderPipelineState(&self.surfaces_pipeline_state);
        // SAFETY: The SurfaceBounds instances match the shader layout and slot.
        // Metal retains the buffers, and writer.finish publishes CPU writes before submission.
        unsafe {
            command_encoder.setVertexBuffer_offset_atIndex(
                Some(&instance_bindings.surfaces.buffer),
                instance_bindings.surfaces.offset,
                SurfaceInputIndex::Surfaces as usize,
            );
        }

        for (index, surface) in surfaces.iter().enumerate() {
            let image_buffer = &surface.image_buffer;
            let texture_size = size(
                DevicePixels::from(CVPixelBufferGetWidth(image_buffer) as i32),
                DevicePixels::from(CVPixelBufferGetHeight(image_buffer) as i32),
            );

            let pixel_format = CVPixelBufferGetPixelFormatType(image_buffer);
            anyhow::ensure!(
                pixel_format == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
                "unsupported surface pixel format: {pixel_format:#x}"
            );

            let plane_texture = |plane, format| -> Result<CFRetained<CVMetalTexture>> {
                let width = CVPixelBufferGetWidthOfPlane(image_buffer, plane);
                let height = CVPixelBufferGetHeightOfPlane(image_buffer, plane);
                let mut texture = ptr::null_mut();
                // SAFETY: The buffer has two planes matching these formats and dimensions, and the output pointer is writable.
                let status = unsafe {
                    CVMetalTextureCache::create_texture_from_image(
                        None,
                        &self.core_video_texture_cache,
                        image_buffer,
                        None,
                        format,
                        width,
                        height,
                        plane,
                        ptr::NonNull::from(&mut texture),
                    )
                };
                anyhow::ensure!(
                    status == kCVReturnSuccess,
                    "creating CoreVideo Metal texture for plane {plane}: status {status}"
                );
                let texture =
                    ptr::NonNull::new(texture).context("retrieving CoreVideo Metal texture")?;
                // SAFETY: The successful create call transfers ownership of this non-null texture.
                Ok(unsafe { CFRetained::from_raw(texture) })
            };
            let y_texture = plane_texture(0, MTLPixelFormat::R8Unorm)?;
            let cb_cr_texture = plane_texture(1, MTLPixelFormat::RG8Unorm)?;
            let y_metal_texture =
                CVMetalTextureGetTexture(&y_texture).context("retrieving luma Metal texture")?;
            let cb_cr_metal_texture = CVMetalTextureGetTexture(&cb_cr_texture)
                .context("retrieving chroma Metal texture")?;

            // SAFETY: The size is copied from a live value, and both planes match their shader texture slots.
            // The frame completion handler retains the CoreVideo wrappers until GPU reads finish.
            unsafe {
                command_encoder.setVertexBytes_length_atIndex(
                    ptr::NonNull::from(&texture_size).cast(),
                    mem::size_of_val(&texture_size),
                    SurfaceInputIndex::TextureSize as usize,
                );
                command_encoder.setFragmentTexture_atIndex(
                    Some(&y_metal_texture),
                    SurfaceInputIndex::YTexture as usize,
                );
                command_encoder.setFragmentTexture_atIndex(
                    Some(&cb_cr_metal_texture),
                    SurfaceInputIndex::CbCrTexture as usize,
                );
            }
            surface_textures.extend([y_texture, cb_cr_texture]);

            // SAFETY: This draws one instance within the uploaded scene's surface range.
            unsafe {
                command_encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                    MTLPrimitiveType::TriangleStrip,
                    0,
                    4,
                    1,
                    first_surface + index,
                );
            }
        }
        Ok(())
    }
}

fn new_command_encoder_for_texture(
    command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
    texture: &ProtocolObject<dyn MTLTexture>,
    depth_texture: &ProtocolObject<dyn MTLTexture>,
    viewport_size: Size<DevicePixels>,
    clear_color: Option<MTLClearColor>,
) -> Result<Retained<ProtocolObject<dyn MTLRenderCommandEncoder>>> {
    let render_pass_descriptor = MTLRenderPassDescriptor::new();
    // SAFETY: Index zero is within the render pass's color attachment array.
    let color_attachment = unsafe {
        render_pass_descriptor
            .colorAttachments()
            .objectAtIndexedSubscript(0)
    };
    color_attachment.setTexture(Some(texture));
    color_attachment.setStoreAction(MTLStoreAction::Store);
    // Without a clear colour the pass draws over what earlier passes left
    // (BenCode's backdrop blur splits a frame into several).
    match clear_color {
        Some(clear_color) => {
            color_attachment.setLoadAction(MTLLoadAction::Clear);
            color_attachment.setClearColor(clear_color);
        }
        None => color_attachment.setLoadAction(MTLLoadAction::Load),
    }

    let depth_attachment = render_pass_descriptor.depthAttachment();
    depth_attachment.setTexture(Some(depth_texture));
    depth_attachment.setLoadAction(MTLLoadAction::Clear);
    depth_attachment.setClearDepth(0.0);
    // The depth buffer is only needed within this pass.
    depth_attachment.setStoreAction(MTLStoreAction::DontCare);

    let command_encoder = command_buffer
        .renderCommandEncoderWithDescriptor(&render_pass_descriptor)
        .context("creating render command encoder")?;
    command_encoder.setViewport(MTLViewport {
        originX: 0.0,
        originY: 0.0,
        width: i32::from(viewport_size.width) as f64,
        height: i32::from(viewport_size.height) as f64,
        znear: 0.0,
        zfar: 1.0,
    });
    Ok(command_encoder)
}

/// Binds what every pipeline of a pass reads: the viewport, the quads' depth
/// scale and how many quads the opaque pass drew (the quad pass draws the
/// opaque quads after them).
fn bind_frame_inputs(
    command_encoder: &ProtocolObject<dyn MTLRenderCommandEncoder>,
    viewport_size: Size<DevicePixels>,
    depth_scale: f32,
    opaque_quads: usize,
) {
    let opaque_quads = opaque_quads as u32;
    // SAFETY: Metal copies the values, which match the vertex shaders'
    // `GlobalInputIndex_ViewportSize`, `GlobalInputIndex_DepthScale` and
    // `QuadInputIndex_QuadCount` arguments.
    unsafe {
        command_encoder.setVertexBytes_length_atIndex(
            ptr::NonNull::from(&viewport_size).cast(),
            mem::size_of_val(&viewport_size),
            GlobalInputIndex::ViewportSize as usize,
        );
        command_encoder.setVertexBytes_length_atIndex(
            ptr::NonNull::from(&depth_scale).cast(),
            mem::size_of_val(&depth_scale),
            GlobalInputIndex::DepthScale as usize,
        );
        command_encoder.setVertexBytes_length_atIndex(
            ptr::NonNull::from(&opaque_quads).cast(),
            mem::size_of_val(&opaque_quads),
            QuadInputIndex::QuadCount as usize,
        );
    }
}

#[cfg(any(test, feature = "bench-support", feature = "test-support"))]
fn read_texture_to_image(texture: &ProtocolObject<dyn MTLTexture>) -> Result<RgbaImage> {
    let width = texture.width() as u32;
    let height = texture.height() as u32;
    let bytes_per_row = width as usize * 4;
    let mut pixels = vec![0u8; height as usize * bytes_per_row];

    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize {
            width: width as usize,
            height: height as usize,
            depth: 1,
        },
    };
    unsafe {
        texture.getBytes_bytesPerRow_fromRegion_mipmapLevel(
            ptr::NonNull::from(pixels.as_mut_slice()).cast(),
            bytes_per_row,
            region,
            0,
        );
    }

    // Convert BGRA to RGBA (swap B and R channels)
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }

    RgbaImage::from_raw(width, height, pixels).context("creating RGBA image from pixel data")
}

fn build_pipeline_state(
    device: &ProtocolObject<dyn MTLDevice>,
    library: &ProtocolObject<dyn MTLLibrary>,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    pixel_format: MTLPixelFormat,
    blending: bool,
) -> Result<Retained<ProtocolObject<dyn MTLRenderPipelineState>>> {
    let vertex_fn = library
        .newFunctionWithName(&NSString::from_str(vertex_fn_name))
        .with_context(|| format!("locating vertex function {vertex_fn_name}"))?;
    let fragment_fn = library
        .newFunctionWithName(&NSString::from_str(fragment_fn_name))
        .with_context(|| format!("locating fragment function {fragment_fn_name}"))?;

    let descriptor = MTLRenderPipelineDescriptor::new();
    descriptor.setLabel(Some(&NSString::from_str(label)));
    descriptor.setVertexFunction(Some(vertex_fn.as_ref()));
    descriptor.setFragmentFunction(Some(fragment_fn.as_ref()));
    // SAFETY: Index zero is within the pipeline's color attachment array.
    let color_attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
    color_attachment.setPixelFormat(pixel_format);
    color_attachment.setBlendingEnabled(blending);
    color_attachment.setRgbBlendOperation(MTLBlendOperation::Add);
    color_attachment.setAlphaBlendOperation(MTLBlendOperation::Add);
    color_attachment.setSourceRGBBlendFactor(MTLBlendFactor::SourceAlpha);
    color_attachment.setSourceAlphaBlendFactor(MTLBlendFactor::One);
    color_attachment.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
    color_attachment.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
    descriptor.setDepthAttachmentPixelFormat(MTLPixelFormat::Depth32Float);

    device
        .newRenderPipelineStateWithDescriptor_error(&descriptor)
        .with_context(|| format!("creating {label} render pipeline"))
}

fn build_depth_stencil_state(
    device: &ProtocolObject<dyn MTLDevice>,
    write_depth: bool,
) -> Result<Retained<ProtocolObject<dyn MTLDepthStencilState>>> {
    let descriptor = MTLDepthStencilDescriptor::new();
    descriptor.setDepthCompareFunction(MTLCompareFunction::Greater);
    descriptor.setDepthWriteEnabled(write_depth);
    device
        .newDepthStencilStateWithDescriptor(&descriptor)
        .context("creating depth stencil state")
}

/// The backdrop composite overwrites its target outright, mixing the
/// original back in itself for the antialiased rounded edge. It draws in a
/// pass of its own, without the depth buffer.
fn build_backdrop_pipeline_state(
    device: &ProtocolObject<dyn MTLDevice>,
    library: &ProtocolObject<dyn MTLLibrary>,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    pixel_format: MTLPixelFormat,
) -> Result<Retained<ProtocolObject<dyn MTLRenderPipelineState>>> {
    let vertex_fn = library
        .newFunctionWithName(&NSString::from_str(vertex_fn_name))
        .with_context(|| format!("locating vertex function {vertex_fn_name}"))?;
    let fragment_fn = library
        .newFunctionWithName(&NSString::from_str(fragment_fn_name))
        .with_context(|| format!("locating fragment function {fragment_fn_name}"))?;
    let descriptor = MTLRenderPipelineDescriptor::new();
    descriptor.setLabel(Some(&NSString::from_str(label)));
    descriptor.setVertexFunction(Some(vertex_fn.as_ref()));
    descriptor.setFragmentFunction(Some(fragment_fn.as_ref()));
    // SAFETY: Index zero is within the pipeline's color attachment array.
    let color_attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
    color_attachment.setPixelFormat(pixel_format);
    color_attachment.setBlendingEnabled(false);
    device
        .newRenderPipelineStateWithDescriptor_error(&descriptor)
        .with_context(|| format!("creating {label} render pipeline"))
}

/// Mark a wallpaper sprite for pane-relative fading and optional Haze.
/// Radius must be positive. GPUI passes the signs through to the sprite shader,
/// which reads the flags and clamps the restored radii to the sprite bounds.
pub fn chat_background_corners(radius: Pixels, empty: bool, haze: bool) -> Corners<Pixels> {
    Corners {
        top_left: -radius,
        top_right: if empty { -radius } else { radius },
        bottom_left: if haze { -radius } else { radius },
        bottom_right: radius,
    }
}

// BenCode: transparent cutout for native views layered behind GPUI (e.g. an
// in-app browser WKWebView). A quad marked with this border colour (with a
// non-transparent dummy fill so `Window::paint_quad` does not cull it) clears
// its clipped pixels to `rgba(0, 0, 0, 0)` via `clear_quad_fragment` so the
// native view behind GPUI shows through while tooltips, popovers and dialogs
// drawn after it composite cleanly on top.
const HOLE_PUNCH_MARKER_HUE: f32 = 0.292_893_23;

/// Marks a quad as a transparent cutout for a native view behind GPUI.
/// `pass_through` enables AppKit `hitTest:` routing through the unobstructed
/// part of the cutout to the native view behind it (`false` while a modal
/// dialog or context menu blocks pointer input).
pub fn hole_punch_marker(pass_through: bool) -> Hsla {
    Hsla {
        h: HOLE_PUNCH_MARKER_HUE,
        s: if pass_through { 1. } else { 0. },
        l: 0.,
        a: 0.,
    }
}

pub fn is_hole_punch(quad: &Quad) -> bool {
    let color = quad.border_color;
    color.h == HOLE_PUNCH_MARKER_HUE && color.a == 0. && color.l == 0.
}

pub fn hole_punch_pass_through(quad: &Quad) -> bool {
    is_hole_punch(quad) && quad.border_color.s > 0.5
}

// BenCode, after MonoCode: frosted glass. A quad marked with this border
// colour (and no border width) asks the renderer to blur the backdrop
// beneath it first.
const BACKDROP_MARKER_HUE: f32 = 0.707_106_77;
const BACKDROP_MAX_SIGMA: f32 = 4096.;

/// The border colour that marks a quad as frosted glass: before drawing it,
/// the renderer blurs what is already drawn behind it by `sigma` device
/// pixels, like CSS `backdrop-filter: blur()`. Its background then tints the
/// glass. Give the quad no border width.
///
/// `tint_alpha` is the alpha of that background as given. An element's
/// opacity fades the background but not the marker (its alpha is 0), so the
/// renderer compares the two and eases the blur with the card: a dialog
/// fading in no longer frosts at full strength from its first frame.
pub fn backdrop_blur_marker(sigma: f32, tint_alpha: f32) -> Hsla {
    Hsla {
        h: BACKDROP_MARKER_HUE,
        s: tint_alpha.clamp(0., 1.),
        l: (sigma / BACKDROP_MAX_SIGMA).clamp(0., 1.),
        a: 0.,
    }
}

/// Below this the blur would not show; the quad is drawn as a plain tint.
const BACKDROP_MIN_SIGMA: f32 = 0.5;

fn backdrop_sigma(quad: &Quad) -> Option<f32> {
    let color = quad.border_color;
    if color.h != BACKDROP_MARKER_HUE || color.a != 0. || color.l <= 0. {
        return None;
    }
    // How far the element's opacity has faded the tint it was given.
    let fade = match quad.background.as_solid() {
        Some(tint) if color.s > 0. => (tint.a / color.s).clamp(0., 1.),
        _ => 1.,
    };
    Some(color.l * BACKDROP_MAX_SIGMA * fade).filter(|sigma| *sigma >= BACKDROP_MIN_SIGMA)
}

/// Whether `piece` is part of the same frosted quad as `quad`. GPUI draws a
/// quad with rounded corners as its interior (corners squared off) and four
/// strips around it, each a copy of the whole quad under its own content
/// mask (`Window::insert_quad`), so the frosts' pieces share bounds and
/// marker.
fn same_frost(piece: &Quad, quad: &Quad) -> bool {
    piece.bounds == quad.bounds
        && piece.border_color == quad.border_color
        && piece.background == quad.background
}

/// The scene's shadows with each frosted card's own drop shadow told the
/// card's bounds, or `None` when nothing is frosted.
///
/// Like CSS `box-shadow`, a drop shadow is not drawn under its element: under
/// an opaque fill that changes nothing, but under frosted glass the backdrop
/// blur would spread it dark across the card. GPUI no longer gives a drop
/// shadow its element's bounds, so the card is the first frosted quad drawn
/// after the shadow that lies inside the shadow's reach. Its bounds go in
/// the drop shadow's `corner_radii`, which GPUI leaves at zero, as origin
/// and size, and its corner radius in `pad`; `shadow_fragment` reads them.
fn frosted_shadows(scene: &Scene) -> Option<Vec<Shadow>> {
    let frosts: Vec<&Quad> = scene
        .quads
        .iter()
        .filter(|quad| backdrop_sigma(quad).is_some())
        .collect();
    if frosts.is_empty() || scene.shadows.is_empty() {
        return None;
    }
    Some(
        scene
            .shadows
            .iter()
            .map(|shadow| {
                let mut shadow = *shadow;
                let drop = shadow.inset == false.into();
                let card = frosts
                    .iter()
                    .filter(|quad| quad.order > shadow.order)
                    .filter(|quad| quad.bounds.is_contained_within(&shadow.bounds))
                    .min_by_key(|quad| quad.order);
                if drop && let Some(card) = card {
                    shadow.corner_radii = Corners {
                        top_left: card.bounds.origin.x,
                        top_right: card.bounds.origin.y,
                        bottom_right: card.bounds.size.width,
                        bottom_left: card.bounds.size.height,
                    };
                    // The card's interior piece has square corners
                    // (`same_frost`); its strips keep the card's radius.
                    let radius = frosts
                        .iter()
                        .filter(|quad| same_frost(quad, card))
                        .map(|quad| quad.corner_radii.top_left.0)
                        .fold(0., f32::max);
                    shadow.pad = radius.to_bits();
                }
                shadow
            })
            .collect(),
    )
}

#[repr(C)]
struct BackdropCompositeParams {
    /// The piece written to.
    origin: [f32; 2],
    size: [f32; 2],
    /// The whole quad, whose rounded edge the piece follows.
    shape_origin: [f32; 2],
    shape_size: [f32; 2],
    region_origin: [f32; 2],
    region_size: [f32; 2],
    viewport_size: [f32; 2],
    corner_radius: f32,
    fallback_alpha: f32,
}

#[derive(Clone)]
struct InstanceBinding {
    buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
    offset: usize,
}

struct InstanceBindings {
    quads: InstanceBinding,
    path_tiles: InstanceBinding,
    path_curves: InstanceBinding,
    path_paints: InstanceBinding,
    shadows: InstanceBinding,
    underlines: InstanceBinding,
    monochrome_sprites: InstanceBinding,
    polychrome_sprites: InstanceBinding,
    surfaces: InstanceBinding,
    native_views: InstanceBinding,
}

fn write_instances(scene: &Scene, writer: &mut InstanceBufferWriter) -> Result<InstanceBindings> {
    let mut path_paints = Vec::with_capacity(scene.paths.len());
    let mut path_curves = Vec::new();
    let mut path_tiles = Vec::new();
    for path in &scene.paths {
        path.append_to(&mut path_paints, &mut path_curves, &mut path_tiles);
    }
    Ok(InstanceBindings {
        quads: writer.write(&scene.quads)?,
        path_tiles: writer.write(&path_tiles)?,
        path_curves: writer.write(&path_curves)?,
        path_paints: writer.write(&path_paints)?,
        shadows: match frosted_shadows(scene) {
            Some(shadows) => writer.write(&shadows)?,
            None => writer.write(&scene.shadows)?,
        },
        underlines: writer.write(&scene.underlines)?,
        monochrome_sprites: writer.write(&scene.monochrome_sprites)?,
        polychrome_sprites: writer.write(&scene.polychrome_sprites)?,
        surfaces: writer.write_iter(scene.surfaces.iter().map(|surface| SurfaceBounds {
            bounds: surface.bounds,
            content_mask: surface.content_mask,
        }))?,
        native_views: writer.write_iter(scene.native_views.iter().map(|native_view| {
            SurfaceBounds {
                bounds: native_view.bounds,
                content_mask: native_view.content_mask,
            }
        }))?,
    })
}

struct InstanceBufferWriter {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    pool: Arc<Mutex<InstanceBufferPool>>,
    unified_memory: bool,
    filled: Vec<(InstanceBuffer, usize)>,
    current: InstanceBuffer,
    offset: usize,
}

impl InstanceBufferWriter {
    fn new(
        device: &ProtocolObject<dyn MTLDevice>,
        pool: &Arc<Mutex<InstanceBufferPool>>,
        unified_memory: bool,
    ) -> Result<Self> {
        let current = pool.lock().acquire(device, unified_memory)?;
        Ok(Self {
            device: device.retain(),
            pool: pool.clone(),
            unified_memory,
            filled: Vec::new(),
            current,
            offset: 0,
        })
    }

    fn allocate<T>(&mut self, count: usize) -> Result<(InstanceBinding, &mut [MaybeUninit<T>])> {
        let size = mem::size_of::<T>() * count;
        let mut offset = self.offset.next_multiple_of(INSTANCE_BUFFER_ALIGNMENT);
        if offset + size > self.current.size {
            self.grow(size)?;
            offset = 0;
        }
        self.offset = offset + size;

        let binding = InstanceBinding {
            buffer: self.current.metal_buffer.clone(),
            offset,
        };
        // SAFETY: The aligned reservation lies within this frame's exclusively owned buffer and does not overlap prior writes.
        let values = unsafe {
            let start = self
                .current
                .metal_buffer
                .contents()
                .as_ptr()
                .cast::<u8>()
                .add(offset);
            slice::from_raw_parts_mut(start.cast::<MaybeUninit<T>>(), count)
        };
        Ok((binding, values))
    }

    fn write<T>(&mut self, values: &[T]) -> Result<InstanceBinding> {
        let (binding, destination) = self.allocate::<T>(values.len())?;
        // SAFETY: The disjoint source and destination hold the same number of T values, and the destination stays MaybeUninit.
        unsafe {
            ptr::copy_nonoverlapping(
                values.as_ptr(),
                destination.as_mut_ptr().cast::<T>(),
                values.len(),
            );
        }
        Ok(binding)
    }

    fn write_iter<T>(
        &mut self,
        values: impl ExactSizeIterator<Item = T>,
    ) -> Result<InstanceBinding> {
        let (binding, destination) = self.allocate::<T>(values.len())?;
        for (slot, value) in destination.iter_mut().zip(values) {
            slot.write(value);
        }
        Ok(binding)
    }

    fn grow(&mut self, required: usize) -> Result<()> {
        let mut pool = self.pool.lock();
        let buffer_size = (pool.buffer_size * 2)
            .max(required.next_power_of_two())
            .min(MAX_INSTANCE_BUFFER_SIZE);
        anyhow::ensure!(
            buffer_size >= required,
            "instance buffer needs {required} bytes, above the maximum of {MAX_INSTANCE_BUFFER_SIZE}"
        );
        anyhow::ensure!(
            buffer_size > self.current.size,
            "frame instance data exceeds the {MAX_INSTANCE_BUFFER_SIZE}-byte maximum"
        );
        if buffer_size != pool.buffer_size {
            log::info!("increased instance buffer size to {buffer_size}");
            pool.reset(buffer_size);
        }
        let buffer = pool.acquire(&self.device, self.unified_memory)?;
        drop(pool);

        let filled = mem::replace(&mut self.current, buffer);
        self.filled.push((filled, self.offset));
        self.offset = 0;
        Ok(())
    }

    fn finish(self) -> InstanceBuffer {
        let Self {
            unified_memory,
            filled,
            current,
            offset,
            ..
        } = self;

        if !unified_memory {
            for (buffer, written) in &filled {
                if *written == 0 {
                    continue;
                }
                buffer.metal_buffer.didModifyRange(NSRange {
                    location: 0,
                    length: *written,
                });
            }
            if offset > 0 {
                current.metal_buffer.didModifyRange(NSRange {
                    location: 0,
                    length: offset,
                });
            }
        }

        // Metal retains encoded resources until the command buffer completes.
        // Only the final, largest buffer is worth keeping in the pool.
        drop(filled);
        current
    }
}

/// Vertex buffer slots bound once per render pass and shared by every
/// pipeline. Each primitive's own slots start after these.
#[repr(C)]
enum GlobalInputIndex {
    ViewportSize = 0,
    DepthScale = 1,
}

#[repr(C)]
enum ShadowInputIndex {
    Shadows = 2,
}

#[repr(C)]
enum QuadInputIndex {
    Quads = 2,
    QuadCount = 3,
}

#[repr(C)]
enum UnderlineInputIndex {
    Underlines = 2,
}

#[repr(C)]
enum SpriteInputIndex {
    Sprites = 2,
    AtlasTextureSize = 3,
    AtlasTexture = 4,
}

#[repr(C)]
enum SurfaceInputIndex {
    Surfaces = 2,
    TextureSize = 3,
    YTexture = 4,
    CbCrTexture = 5,
}

#[repr(C)]
enum PathTileInputIndex {
    Tiles = 2,
    Curves = 3,
    Paints = 4,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct SurfaceBounds {
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
}

#[cfg(any(test, feature = "bench-support", feature = "test-support"))]
pub struct MetalHeadlessRenderer {
    renderer: MetalRenderer,
}

#[cfg(any(test, feature = "bench-support", feature = "test-support"))]
impl MetalHeadlessRenderer {
    pub fn new() -> Result<Self> {
        let instance_buffer_pool = Arc::new(Mutex::new(InstanceBufferPool::default()));
        let renderer = MetalRenderer::new_headless(instance_buffer_pool)?;
        Ok(Self { renderer })
    }
}

#[cfg(any(test, feature = "bench-support", feature = "test-support"))]
impl gpui::PlatformHeadlessRenderer for MetalHeadlessRenderer {
    fn render_scene_to_image(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> anyhow::Result<image::RgbaImage> {
        self.renderer.render_scene_to_image(scene, size)
    }

    fn render_scene(&mut self, scene: &Scene, size: Size<DevicePixels>) -> anyhow::Result<()> {
        self.renderer.render_scene(scene, size)
    }

    fn sprite_atlas(&self) -> Arc<dyn gpui::PlatformAtlas> {
        self.renderer.sprite_atlas().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Point;
    use objc2_core_foundation::{CFDictionary, CFType};
    use objc2_core_video::{
        CVPixelBufferCreate, CVPixelBufferGetBaseAddressOfPlane,
        CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
        CVPixelBufferUnlockBaseAddress, kCVPixelBufferIOSurfacePropertiesKey,
    };
    use objc2_metal::MTLCommandBufferStatus;

    #[test]
    fn render_to_image_rejects_zero_sized_layer() -> Result<()> {
        let mut renderer = MetalRenderer::new(Default::default(), false, false)?;
        for (width, height) in [(0, 0), (0, 64), (64, 0)] {
            renderer.update_drawable_size(size(DevicePixels(width), DevicePixels(height)));
            assert!(renderer.render_to_image(&Scene::default()).is_err());
        }
        Ok(())
    }

    #[test]
    fn display_link_mode_consumes_and_clears_drawables() -> Result<()> {
        let mut renderer = MetalRenderer::new(Default::default(), false, false)?;
        renderer.update_drawable_size(size(DevicePixels(64), DevicePixels(64)));
        let drawable = renderer
            .layer()
            .and_then(|layer| layer.nextDrawable())
            .context("window renderer must supply a drawable")?;

        renderer.set_display_link_attached(true);
        assert!(matches!(
            renderer.display_link_mode,
            DisplayLinkMode::Attached { drawable: None }
        ));
        assert!(
            renderer
                .set_frame_drawable(Some(drawable.clone()))
                .is_none()
        );
        assert!(matches!(
            renderer.display_link_mode,
            DisplayLinkMode::Attached { drawable: Some(_) }
        ));
        renderer.draw(&Scene::default());
        assert!(matches!(
            renderer.display_link_mode,
            DisplayLinkMode::Attached { drawable: None }
        ));
        // A frame that drew consumed its drawable; one that didn't hands it back.
        assert!(
            renderer
                .set_frame_drawable(Some(drawable.clone()))
                .is_none()
        );
        assert!(renderer.set_frame_drawable(None).is_some());

        renderer.set_frame_drawable(Some(drawable));
        renderer.set_display_link_attached(false);
        assert!(matches!(
            renderer.display_link_mode,
            DisplayLinkMode::Detached
        ));
        Ok(())
    }

    #[test]
    fn render_core_video_surface() {
        let surface_properties_key = unsafe { kCVPixelBufferIOSurfacePropertiesKey };
        let attributes = CFDictionary::from_slices(
            &[surface_properties_key],
            &[&*CFDictionary::<CFType, CFType>::empty()],
        );
        let mut image_buffer = ptr::null_mut();
        assert_eq!(
            unsafe {
                CVPixelBufferCreate(
                    None,
                    16,
                    16,
                    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
                    Some(attributes.as_opaque()),
                    ptr::NonNull::from(&mut image_buffer),
                )
            },
            kCVReturnSuccess
        );
        let image_buffer = ptr::NonNull::new(image_buffer).unwrap();
        let image_buffer = unsafe { CFRetained::from_raw(image_buffer) };
        unsafe {
            assert_eq!(
                CVPixelBufferLockBaseAddress(&image_buffer, CVPixelBufferLockFlags::empty()),
                kCVReturnSuccess
            );
            for plane in 0..2 {
                let bytes = CVPixelBufferGetBaseAddressOfPlane(&image_buffer, plane).cast::<u8>();
                let length = CVPixelBufferGetBytesPerRowOfPlane(&image_buffer, plane)
                    * CVPixelBufferGetHeightOfPlane(&image_buffer, plane);
                slice::from_raw_parts_mut(bytes, length).fill(128);
            }
            assert_eq!(
                CVPixelBufferUnlockBaseAddress(&image_buffer, CVPixelBufferLockFlags::empty()),
                kCVReturnSuccess
            );
        }

        let bounds = Bounds {
            origin: Point::default(),
            size: size(ScaledPixels(16.0), ScaledPixels(16.0)),
        };
        let mut scene = Scene::default();
        scene.insert_primitive(PaintSurface {
            order: 0,
            bounds,
            content_mask: ContentMask { bounds },
            image_buffer: image_buffer.clone(),
        });
        drop(image_buffer);
        scene.finish();

        let mut renderer =
            MetalRenderer::new_headless(Arc::new(Mutex::new(InstanceBufferPool::default())))
                .unwrap();
        let image = renderer
            .render_scene_to_image(&scene, size(DevicePixels(16), DevicePixels(16)))
            .unwrap();
        for image::Rgba([red, green, blue, alpha]) in image.pixels() {
            assert!(red.abs_diff(128) <= 2);
            assert!(green.abs_diff(128) <= 2);
            assert!(blue.abs_diff(128) <= 2);
            assert_eq!(*alpha, 255);
        }

        let descriptor = MTLTextureDescriptor::new();
        unsafe {
            descriptor.setWidth(16);
            descriptor.setHeight(16);
        }
        descriptor.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        descriptor.setUsage(MTLTextureUsage::RenderTarget);
        let uses_shared_storage = cfg!(target_os = "ios") || renderer.is_apple_gpu;
        descriptor.setStorageMode(if uses_shared_storage {
            MTLStorageMode::Shared
        } else {
            MTLStorageMode::Managed
        });
        let target_texture = renderer
            .device
            .newTextureWithDescriptor(&descriptor)
            .unwrap();
        let command_buffer = renderer
            .render_frame(
                &scene,
                &target_texture,
                size(DevicePixels(16), DevicePixels(16)),
            )
            .unwrap();
        if !uses_shared_storage {
            let blit = command_buffer.blitCommandEncoder().unwrap();
            blit.synchronizeResource(ProtocolObject::from_ref(&*target_texture));
            blit.endEncoding();
        }
        drop(scene);
        drop(renderer);
        command_buffer.commit();
        command_buffer.waitUntilCompleted();
        assert_eq!(command_buffer.status(), MTLCommandBufferStatus::Completed);
        assert_eq!(read_texture_to_image(&target_texture).unwrap(), image);
    }
}
