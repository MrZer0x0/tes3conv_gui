#![windows_subsystem = "windows"]
#[allow(non_snake_case)]

use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::UI::WindowsAndMessaging::*,
    Win32::Graphics::Gdi::*,
    Win32::UI::Controls::*,
    Win32::UI::Controls::Dialogs::*,
    Win32::UI::Input::KeyboardAndMouse::*,
    Win32::System::LibraryLoader::GetModuleHandleW,
};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::path::PathBuf;
use std::collections::HashMap;
use thiserror::Error;
use serde_json;
use tes3::esp::Plugin;

#[allow(non_snake_case)]
#[inline]
fn LOWORD(l: u32) -> u16 {
    (l & 0xffff) as u16
}

#[allow(non_snake_case)]
#[inline]
fn HIWORD(l: u32) -> u16 {
    ((l >> 16) & 0xffff) as u16
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Windows error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("File exists: {0}")]
    FileExists(PathBuf),
    #[error("Channel error: {0}")]
    Channel(#[from] std::sync::mpsc::SendError<f32>),
}

#[derive(Copy, Clone, PartialEq)]
enum ConversionType {
    ToJson,
    ToPlugin,
}

#[derive(Clone)]
struct Localizer {
    russian_to_1c: HashMap<char, char>,
    c1_to_russian: HashMap<char, char>,
}

impl Localizer {
    fn new() -> Self {
        let mut russian_to_1c = HashMap::new();
        let mappings = [
            ('А', 'À'), ('Б', 'Á'), ('В', 'Â'), ('Г', 'Ã'), ('Д', 'Ä'),
            ('Е', 'Å'), ('Ж', 'Æ'), ('З', 'Ç'), ('И', 'È'), ('Й', 'É'),
            ('К', 'Ê'), ('Л', 'Ë'), ('М', 'Ì'), ('Н', 'Í'), ('О', 'Î'),
            ('П', 'Ï'), ('Р', 'Ð'), ('С', 'Ñ'), ('Т', 'Ò'), ('У', 'Ó'),
            ('Ф', 'Ô'), ('Х', 'Õ'), ('Ц', 'Ö'), ('Ч', '×'), ('Ш', 'Ø'),
            ('Щ', 'Ù'), ('Ъ', 'Ú'), ('Ы', 'Û'), ('Ь', 'Ü'), ('Э', 'Ý'),
            ('Ю', 'Þ'), ('Я', 'ß'), ('а', 'à'), ('б', 'á'), ('в', 'â'),
            ('г', 'ã'), ('д', 'ä'), ('е', 'å'), ('ж', 'æ'), ('з', 'ç'),
            ('и', 'è'), ('й', 'é'), ('к', 'ê'), ('л', 'ë'), ('м', 'ì'),
            ('н', 'í'), ('о', 'î'), ('п', 'ï'), ('р', 'ð'), ('с', 'ñ'),
            ('т', 'ò'), ('у', 'ó'), ('ф', 'ô'), ('х', 'õ'), ('ц', 'ö'),
            ('ч', '÷'), ('ш', 'ø'), ('щ', 'ù'), ('ъ', 'ú'), ('ы', 'û'),
            ('ь', 'ü'), ('э', 'ý'), ('ю', 'þ'), ('я', 'ÿ'), ('ё', '¸'),
            ('Ё', '¨')
        ];

        for (rus, old) in mappings.iter() {
            russian_to_1c.insert(*rus, *old);
        }

        let mut c1_to_russian = HashMap::new();
        for (k, v) in russian_to_1c.iter() {
            c1_to_russian.insert(*v, *k);
        }

        Self {
            russian_to_1c,
            c1_to_russian,
        }
    }

    fn to_russian(&self, text: &str) -> String {
        text.chars()
            .map(|c| *self.c1_to_russian.get(&c).unwrap_or(&c))
            .collect()
    }

    fn from_russian(&self, text: &str) -> String {
        text.chars()
            .map(|c| *self.russian_to_1c.get(&c).unwrap_or(&c))
            .collect()
    }
}

const ID_INPUT_PATH: i32 = 101;
const ID_BROWSE_BUTTON: i32 = 102;
const ID_RADIO_TO_JSON: i32 = 103;
const ID_RADIO_TO_PLUGIN: i32 = 104;
const ID_CHECK_1C: i32 = 105;
const ID_CHECK_COMPACT: i32 = 106;
const ID_CHECK_OVERWRITE: i32 = 107;
const ID_CONVERT_BUTTON: i32 = 108;
const ID_PROGRESS_BAR: i32 = 109;
const ID_INFO_TEXT: i32 = 110;
struct ConverterWindow {
    hwnd: HWND,
    controls: HashMap<i32, HWND>,
    input_path: String,
    conversion_type: ConversionType,
    use_compact: bool,
    overwrite: bool,
    use_1c_localization: bool,
    progress: i32,
    is_converting: bool,
    progress_rx: Option<Receiver<f32>>,
    progress_tx: Option<Sender<f32>>,
    localizer: Localizer,
}

impl ConverterWindow {
    fn new() -> Result<Self> {
        let (tx, rx) = channel();
        
        Ok(Self {
            hwnd: HWND(0),
            controls: HashMap::new(),
            input_path: String::new(),
            conversion_type: ConversionType::ToJson,
            use_compact: false,
            overwrite: false,
            use_1c_localization: true,
            progress: 0,
            is_converting: false,
            progress_rx: Some(rx),
            progress_tx: Some(tx),
            localizer: Localizer::new(),
        })
    }

unsafe fn create_window(&mut self) -> Result<()> {
    let instance = GetModuleHandleW(None)?;
    
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(Self::wndproc),
        hInstance: instance,
        hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as isize),
        lpszClassName: w!("TES3ConverterWindow"),
        ..Default::default()
    };

    RegisterClassExW(&wc);

    // Стиль окна без возможности изменения размера
    let style = WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_VISIBLE.0;

    self.hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        w!("TES3ConverterWindow"),
        w!("tes3conv GUI v0.2 (+1C localisation decoder)"),
        WINDOW_STYLE(style),
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        500,  // Увеличенная ширина
        420,  // Увеличенная высота
        HWND::default(),
        HMENU::default(),
        instance,
        Some(self as *mut _ as _),
    );

    if self.hwnd.0 == 0 {
        return Err(Error::Windows(windows::core::Error::from_win32()));
    }

    self.create_controls()?;
    Ok(())
}


    unsafe fn create_controls(&mut self) -> Result<()> {
        let create_window = |class_name, window_name, style, x, y, width, height, id| -> Result<HWND> {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                window_name,
                style,
                x, y, width, height,
                self.hwnd,
                HMENU(id as isize),
                GetModuleHandleW(None)?,
                None,
            );
            
            if hwnd.0 == 0 {
                return Err(Error::Windows(windows::core::Error::from_win32()));
            }
            Ok(hwnd)
        };

        let edit_style = WINDOW_STYLE(
            WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | (ES_AUTOHSCROLL as u32)
        );
        
        let button_style = WINDOW_STYLE(
            WS_CHILD.0 | WS_VISIBLE.0 | (BS_PUSHBUTTON as u32)
        );
        
        let radio_style = WINDOW_STYLE(
            WS_CHILD.0 | WS_VISIBLE.0 | (BS_AUTORADIOBUTTON as u32)
        );
        
        let check_style = WINDOW_STYLE(
            WS_CHILD.0 | WS_VISIBLE.0 | (BS_AUTOCHECKBOX as u32)
        );
        
        let progress_style = WINDOW_STYLE(
            WS_CHILD.0 | WS_VISIBLE.0
        );

        // Input field
        let hwnd = create_window(
            w!("EDIT"),
            w!(""),
            edit_style,
            10, 10, 380, 25,
            ID_INPUT_PATH
        )?;
        self.controls.insert(ID_INPUT_PATH, hwnd);

        // Browse button
        let hwnd = create_window(
            w!("BUTTON"),
            w!("..."),
            button_style,
            400, 10, 30, 25,
            ID_BROWSE_BUTTON
        )?;
        self.controls.insert(ID_BROWSE_BUTTON, hwnd);

        // Radio buttons
        let radio_style_with_group = WINDOW_STYLE(radio_style.0 | WS_GROUP.0);
        let hwnd = create_window(
            w!("BUTTON"),
            w!("ESP/ESM to JSON"),
            radio_style_with_group,
            10, 50, 200, 25,
            ID_RADIO_TO_JSON
        )?;
        self.controls.insert(ID_RADIO_TO_JSON, hwnd);

        let hwnd = create_window(
            w!("BUTTON"),
            w!("JSON to ESP/ESM"),
            radio_style,
            220, 50, 200, 25,
            ID_RADIO_TO_PLUGIN
        )?;
        self.controls.insert(ID_RADIO_TO_PLUGIN, hwnd);

        // Checkboxes
        let hwnd = create_window(
            w!("BUTTON"),
            w!("Use 1C localization"),
            check_style,
            10, 90, 250, 25,
            ID_CHECK_1C
        )?;
        self.controls.insert(ID_CHECK_1C, hwnd);

        let hwnd = create_window(
            w!("BUTTON"),
            w!("Compact JSON"),
            check_style,
            10, 120, 250, 25,
            ID_CHECK_COMPACT
        )?;
        self.controls.insert(ID_CHECK_COMPACT, hwnd);

        let hwnd = create_window(
            w!("BUTTON"),
            w!("Overwrite files"),
            check_style,
            10, 150, 250, 25,
            ID_CHECK_OVERWRITE
        )?;
        self.controls.insert(ID_CHECK_OVERWRITE, hwnd);

        // Progress bar
        let hwnd = create_window(
            PROGRESS_CLASS,
            w!(""),
            progress_style,
            10, 190, 410, 25,
            ID_PROGRESS_BAR
        )?;
        self.controls.insert(ID_PROGRESS_BAR, hwnd);
        SendMessageW(hwnd, PBM_SETRANGE32, WPARAM(0), LPARAM(100));

        // Convert button
        let hwnd = create_window(
            w!("BUTTON"),
            w!("Convert"),
            button_style,
            10, 230, 410, 30,
            ID_CONVERT_BUTTON
        )?;
        self.controls.insert(ID_CONVERT_BUTTON, hwnd);

        // Info text
        let hwnd = create_window(
            w!("STATIC"),
            w!("tes3conv gui + 1c decoder v0.2\r\nautor: MrZer0 (t.me/morrowind24)\r\n\r\ntes3conv v0.4\r\nautor: Greatness7 (github.com/Greatness7/tes3conv)"),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0),
            10, 270, 410, 80,
            ID_INFO_TEXT
        )?;
        self.controls.insert(ID_INFO_TEXT, hwnd);

        // Initial values
        SendMessageW(
            self.controls[&ID_RADIO_TO_JSON],
            BM_SETCHECK,
            WPARAM(1),
            LPARAM(0)
        );
        SendMessageW(
            self.controls[&ID_CHECK_1C],
            BM_SETCHECK,
            WPARAM(1),
            LPARAM(0)
        );

        Ok(())
    }

    unsafe fn handle_command(&mut self, cmd: i32, notification: u16) {
        match cmd {
            ID_BROWSE_BUTTON => {
                if let Some(path) = self.show_file_dialog() {
                    self.input_path = path;
                    SetWindowTextW(
                        self.controls[&ID_INPUT_PATH],
                        &HSTRING::from(self.input_path.as_str()),
                    );
                }
            }
            ID_CONVERT_BUTTON => {
                if !self.is_converting {
                    self.start_conversion();
                }
            }
            ID_RADIO_TO_JSON => {
                if notification == BN_CLICKED as u16 {
                    self.conversion_type = ConversionType::ToJson;
                }
            }
            ID_RADIO_TO_PLUGIN => {
                if notification == BN_CLICKED as u16 {
                    self.conversion_type = ConversionType::ToPlugin;
                }
            }
            ID_CHECK_1C => {
                if notification == BN_CLICKED as u16 {
                    self.use_1c_localization = SendMessageW(
                        self.controls[&ID_CHECK_1C],
                        BM_GETCHECK,
                        WPARAM(0),
                        LPARAM(0)
                    ).0 == 1;
                }
            }
            ID_CHECK_COMPACT => {
                if notification == BN_CLICKED as u16 {
                    self.use_compact = SendMessageW(
                        self.controls[&ID_CHECK_COMPACT],
                        BM_GETCHECK,
                        WPARAM(0),
                        LPARAM(0)
                    ).0 == 1;
                }
            }
            ID_CHECK_OVERWRITE => {
                if notification == BN_CLICKED as u16 {
                    self.overwrite = SendMessageW(
                        self.controls[&ID_CHECK_OVERWRITE],
                        BM_GETCHECK,
                        WPARAM(0),
                        LPARAM(0)
                    ).0 == 1;
                }
            }
            _ => {}
        }
    }

    fn show_file_dialog(&self) -> Option<String> {
        unsafe {
            let mut buffer = [0u16; MAX_PATH as usize];
            let mut ofn = OPENFILENAMEW {
                lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
                hwndOwner: self.hwnd,
                lpstrFilter: w!("TES3 Files\0*.esp;*.esm;*.json\0All Files\0*.*\0"),
                lpstrFile: PWSTR(buffer.as_mut_ptr()),
                nMaxFile: MAX_PATH,
                Flags: OFN_PATHMUSTEXIST | OFN_FILEMUSTEXIST,
                ..Default::default()
            };

            if GetOpenFileNameW(&mut ofn).as_bool() {
                let len = buffer.iter().position(|&x| x == 0).unwrap_or(0);
                Some(String::from_utf16_lossy(&buffer[..len]))
            } else {
                None
            }
        }
    }

    fn start_conversion(&mut self) {
        self.is_converting = true;
        self.progress = 0;
        
        unsafe {
            SendMessageW(
                self.controls[&ID_PROGRESS_BAR],
                PBM_SETPOS,
                WPARAM(0),
                LPARAM(0)
            );
            EnableWindow(self.controls[&ID_CONVERT_BUTTON], false);
        }

        let input_path = self.input_path.clone();
        let conversion_type = self.conversion_type;
        let use_compact = self.use_compact;
        let overwrite = self.overwrite;
        let use_1c_localization = self.use_1c_localization;
        let tx = self.progress_tx.clone().unwrap();
        let localizer = self.localizer.clone();

        std::thread::spawn(move || {
            let result = convert_file(
                &input_path,
                conversion_type,
                use_compact,
                overwrite,
                use_1c_localization,
                &localizer,
                tx.clone(),
            );

            if let Err(e) = result {
                log::error!("Conversion error: {}", e);
                tx.send(-1.0).unwrap_or_default();
            }
        });
    }

    unsafe fn update_progress(&mut self) {
        if let Some(rx) = &self.progress_rx {
            if let Ok(progress) = rx.try_recv() {
                if progress < 0.0 {
                    self.is_converting = false;
                    EnableWindow(self.controls[&ID_CONVERT_BUTTON], true);
                    MessageBoxW(
                        self.hwnd,
                        w!("File conversion error"),
                        w!("Error"),
                        MB_OK | MB_ICONERROR,
                    );
                } else {
                    self.progress = progress as i32;
                    SendMessageW(
                        self.controls[&ID_PROGRESS_BAR],
                        PBM_SETPOS,
                        WPARAM(self.progress as _),
                        LPARAM(0),
                    );
                    
                    if progress >= 100.0 {
                        self.is_converting = false;
                        EnableWindow(self.controls[&ID_CONVERT_BUTTON], true);
                        MessageBoxW(
                            self.hwnd,
                            w!("Conversion completed successfully"),
                            w!("Done"),
                            MB_OK | MB_ICONINFORMATION,
                        );
                    }
                }
            }
        }
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let this = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Self;

        match msg {
            WM_CREATE => {
                let create_struct = lparam.0 as *const CREATESTRUCTW;
                let this = (*create_struct).lpCreateParams as *mut Self;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, this as _);
                LRESULT(0)
            }
            WM_COMMAND => {
                if !this.is_null() {
                    (*this).handle_command(
                        LOWORD(wparam.0 as u32) as i32,
                        HIWORD(wparam.0 as u32) as u16,
                    );
                }
                LRESULT(0)
            }
            WM_TIMER => {
                if !this.is_null() {
                    (*this).update_progress();
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    fn run(&mut self) -> Result<()> {
        unsafe {
            self.create_window()?;
            SetTimer(self.hwnd, 1, 100, None);

            let mut message = MSG::default();
            while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(())
    }
}

fn convert_file(
    input_path: &str,
    conversion_type: ConversionType,
    use_compact: bool,
    overwrite: bool,
    use_1c_localization: bool,
    localizer: &Localizer,
    progress_tx: Sender<f32>,
) -> Result<()> {
    let input_path = PathBuf::from(input_path);
    let output_path = match conversion_type {
        ConversionType::ToJson => input_path.with_extension("json"),
        ConversionType::ToPlugin => input_path.with_extension("esp"),
    };

    if output_path.exists() && !overwrite {
        return Err(Error::FileExists(output_path));
    }

    match conversion_type {
        ConversionType::ToJson => {
            let mut plugin = Plugin::new();
            plugin.load_path(&input_path)?;
            progress_tx.send(33.0)?;

            let json = if use_compact {
                serde_json::to_string(&plugin.objects)?
            } else {
                serde_json::to_string_pretty(&plugin.objects)?
            };

            let json = if use_1c_localization {
                localizer.to_russian(&json)
            } else {
                json
            };

            std::fs::write(output_path, json)?;
            progress_tx.send(100.0)?;
        }
        ConversionType::ToPlugin => {
            let contents = std::fs::read_to_string(&input_path)?;
            progress_tx.send(33.0)?;

            let json = if use_1c_localization {
                localizer.from_russian(&contents)
            } else {
                contents
            };

            let mut plugin = Plugin::new();
            plugin.objects = serde_json::from_str(&json)?;
            progress_tx.send(66.0)?;

            plugin.save_path(output_path)?;
            progress_tx.send(100.0)?;
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let mut window = ConverterWindow::new()?;
    window.run()
}
