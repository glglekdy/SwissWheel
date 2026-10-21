//! A Direct2D swap chain shown through Windows.UI.Composition, over one or
//! more host-backdrop blurs (the glass) clipped to rounded rectangles. Used
//! by the progress card and the desktop menu; the wheel has its own.

use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{CompositionRoundedRectangleGeometry, Compositor, SpriteVisual};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::WinRT::Composition::{ICompositorDesktopInterop, ICompositorInterop};
use windows::core::{HSTRING, Interface, PCWSTR, Result, w};
use windows_numerics::Vector2;

use crate::wheel::render::{create_d3d_device, icon_font};

pub struct Surface {
    _device: ID3D11Device,
    ctx: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    _compositor: Compositor,
    _target: DesktopWindowTarget,
    /// Blurs of whatever is behind the window, each clipped to its shape.
    glass: Vec<(SpriteVisual, CompositionRoundedRectangleGeometry)>,
    _content: SpriteVisual,
    dwrite: IDWriteFactory,
    /// Family of the system icon font (see `wheel::glyph`).
    icon_font: PCWSTR,
    size: (u32, u32),
}

impl Surface {
    /// `glass_regions`: how many separately shaped glass areas the window has
    /// (all start hidden; see `set_glass`).
    pub fn new(hwnd: HWND, width: u32, height: u32, glass_regions: usize) -> Result<Surface> {
        unsafe {
            let device = create_d3d_device()?;
            let dxgi_device: IDXGIDevice = device.cast()?;
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let ctx = factory
                .CreateDevice(&dxgi_device)?
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let dxgi_factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                ..Default::default()
            };
            let swap = dxgi_factory.CreateSwapChainForComposition(&device, &desc, None)?;
            let buffer: IDXGISurface = swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            };
            ctx.SetTarget(&ctx.CreateBitmapFromDxgiSurface(&buffer, Some(&props))?);

            // Needs a DispatcherQueue on this thread (created in app::run).
            let compositor = Compositor::new()?;
            let target = compositor
                .cast::<ICompositorDesktopInterop>()?
                .CreateDesktopWindowTarget(hwnd, true)?;
            let full = Vector2 {
                X: width as f32,
                Y: height as f32,
            };
            let root = compositor.CreateContainerVisual()?;
            root.SetSize(full)?;
            target.SetRoot(&root)?;
            let children = root.Children()?;
            let mut glass = Vec::with_capacity(glass_regions);
            for _ in 0..glass_regions {
                let backdrop = compositor.CreateSpriteVisual()?;
                backdrop.SetSize(full)?;
                backdrop.SetBrush(&compositor.CreateHostBackdropBrush()?)?;
                let clip = compositor.CreateRoundedRectangleGeometry()?;
                backdrop.SetClip(&compositor.CreateGeometricClipWithGeometry(&clip)?)?;
                backdrop.SetIsVisible(false)?;
                children.InsertAtBottom(&backdrop)?;
                glass.push((backdrop, clip));
            }
            let content = compositor.CreateSpriteVisual()?;
            content.SetSize(full)?;
            let surface = compositor
                .cast::<ICompositorInterop>()?
                .CreateCompositionSurfaceForSwapChain(&swap)?;
            content.SetBrush(&compositor.CreateSurfaceBrushWithSurface(&surface)?)?;
            children.InsertAtTop(&content)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            Ok(Surface {
                icon_font: icon_font(&dwrite),
                _device: device,
                ctx,
                swap,
                _compositor: compositor,
                _target: target,
                glass,
                _content: content,
                dwrite,
                size: (width, height),
            })
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Shows glass area `index` behind `rect`, or hides it (`None`).
    pub fn set_glass(&self, index: usize, rect: Option<D2D_RECT_F>, corner: f32) -> Result<()> {
        let Some((backdrop, clip)) = self.glass.get(index) else {
            return Ok(());
        };
        let Some(rect) = rect else {
            return backdrop.SetIsVisible(false);
        };
        clip.SetOffset(Vector2 {
            X: rect.left,
            Y: rect.top,
        })?;
        clip.SetSize(Vector2 {
            X: rect.right - rect.left,
            Y: rect.bottom - rect.top,
        })?;
        clip.SetCornerRadius(Vector2 {
            X: corner,
            Y: corner,
        })?;
        backdrop.SetIsVisible(true)
    }

    pub fn ctx(&self) -> &ID2D1DeviceContext {
        &self.ctx
    }

    pub fn icon_font(&self) -> PCWSTR {
        self.icon_font
    }

    pub fn present(&self) -> Result<()> {
        unsafe { self.swap.Present(1, DXGI_PRESENT(0)).ok() }
    }

    /// Single-line text, centered vertically, cut with "…" when too long.
    pub fn text_format(
        &self,
        family: PCWSTR,
        size: f32,
        weight: DWRITE_FONT_WEIGHT,
        align: DWRITE_TEXT_ALIGNMENT,
    ) -> Result<IDWriteTextFormat> {
        unsafe {
            let format = self.dwrite.CreateTextFormat(
                family,
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("ko-kr"),
            )?;
            format.SetTextAlignment(align)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let sign = self.dwrite.CreateEllipsisTrimmingSign(&format)?;
            format.SetTrimming(
                &DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                },
                &sign,
            )?;
            Ok(format)
        }
    }
}

pub fn rounded(rect: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect,
        radiusX: radius,
        radiusY: radius,
    }
}

pub fn inset(rect: D2D_RECT_F, d: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: rect.left + d,
        top: rect.top + d,
        right: rect.right - d,
        bottom: rect.bottom - d,
    }
}

pub fn draw_text(
    ctx: &ID2D1DeviceContext,
    text: &str,
    format: &IDWriteTextFormat,
    rect: D2D_RECT_F,
    brush: &ID2D1SolidColorBrush,
) {
    unsafe {
        ctx.DrawText(
            &HSTRING::from(text),
            format,
            &rect,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
}
