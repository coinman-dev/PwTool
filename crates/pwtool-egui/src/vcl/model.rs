//! The state of a form: every VCL control of the original form description
//! with its current state (text, check mark, items…). The window logic
//! reads and changes controls by name, like in the original; drawing
//! (`render`) only records [`Event`]s, which the logic handles afterwards.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use egui::{Color32, Rect, Vec2, pos2, vec2};
use pwtool_app::form::FormObj;
use pwtool_core::language::trl;
use zeroize::Zeroizing;

use super::text::TextBuf;

fn byte_of(s: &str, char_index: usize) -> usize {
    s.char_indices().nth(char_index).map_or(s.len(), |(i, _)| i)
}

pub const AK_LEFT: u8 = 1;
pub const AK_TOP: u8 = 2;
pub const AK_RIGHT: u8 = 4;
pub const AK_BOTTOM: u8 = 8;

/// `TAlign`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Top,
    Bottom,
    Left,
    Right,
    Client,
}

/// Something the user did; handled by the window logic after drawing.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Button, speed button, tool button, check box, radio button clicked.
    Click(String),
    /// Text, selection or value of a control changed by the user.
    Change(String),
    /// Menu item chosen (also by its shortcut).
    Menu(String),
    /// Link of a `TLinkLabel` clicked: (control, URL).
    Link(String, String),
    /// Double click on a list.
    DblClick(String),
    /// Enter pressed in a single-line edit or combo box.
    Enter(String),
    /// A control lost the keyboard focus (`OnExit`).
    Exit(String),
}

/// An image of an image list: key of `pwtool_app::icons` and display size.
#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    pub form: String,
    pub collection: String,
    pub name: String,
    pub size: f32,
}

/// Font of a control relative to the form font.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub mono: bool,
    /// Size relative to the form font.
    pub scale: f32,
    pub color: Option<Color32>,
    /// Named font family (`fonts::PASSW`) instead of the form font.
    pub family: Option<&'static str>,
}

/// Kind-specific state of a control.
pub enum State {
    Label { halign: f32, wrap: bool, vcenter: bool, focus_control: Option<String> },
    Link,
    Button { default: bool, cancel: bool },
    Speed { toggle: bool, down: bool, flat: bool, icon: Option<Icon>, show_caption: bool },
    Check { checked: bool },
    Radio { checked: bool },
    Edit { text: TextBuf, password: bool, readonly: bool, max_len: usize, halign: f32 },
    Memo { text: TextBuf, readonly: bool, wrap: bool },
    Combo { items: Vec<String>, index: Option<usize>, text: TextBuf, editable: bool },
    UpDown { min: i64, max: i64, inc: i64, pos: i64, associate: Option<String> },
    Group,
    RadioGroup { items: Vec<String>, index: Option<usize>, columns: usize },
    Panel { bevel: bool },
    Sheet,
    Pages { active: usize },
    /// `marks`: selected items of a `MultiSelect` list (`None` = single selection).
    ListBox { items: Vec<String>, sel: Option<usize>, marks: Option<Vec<bool>> },
    CheckList { items: Vec<String>, checked: Vec<bool>, sel: Option<usize> },
    Status { text: String },
    Track { min: i64, max: i64, pos: i64 },
    Progress { pos: i64, max: i64 },
    Date { ymd: (i32, u32, u32), checked: bool, show_check: bool },
    ToolBar,
    ToolButton { icon: Option<Icon>, separator: bool, down: bool, toggle: bool },
    Splitter,
    Bevel { shape: String },
    Image { png: Option<Vec<u8>>, version: u32, stretch: bool, center: bool },
    /// Drawn by the window logic (`Form::set_painter`): list views, grids,
    /// the strength meter.
    Custom,
}

pub struct Control {
    pub name: String,
    pub class: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Design rectangle (scaled to points), relative to the parent's client area.
    pub design: Rect,
    /// Design size of the client area for containers (scaled).
    pub design_client: Vec2,
    pub anchors: u8,
    pub align: Align,
    /// `TLabel.AutoSize`: width follows the text; horizontal alignment kept.
    pub autosize: Option<f32>,
    /// Caption with `&` mnemonic, translated.
    pub caption: String,
    pub hint: String,
    pub visible: bool,
    pub enabled: bool,
    pub font: FontStyle,
    pub popup_menu: Option<String>,
    pub tab_order: i64,
    pub state: State,
    /// Rectangle on screen after the last layout.
    pub rect: Rect,
    /// Text field needs the focus at the next frame.
    pub focus_request: bool,
    pub has_focus: bool,
    /// Selection of an edit or memo after the last frame (character
    /// indices, start <= end; equal = cursor position).
    pub selection: Option<(usize, usize)>,
    /// Changes of the cursor/selection to apply at the next frame.
    pub edit_ops: Vec<EditOp>,
}

/// Operation on the selection of an edit control.
#[derive(Clone, Debug, PartialEq)]
pub enum EditOp {
    SelectAll,
    /// Place the cursor at a character index.
    Cursor(usize),
    Undo,
}

/// One entry of a menu tree.
#[derive(Clone, Debug)]
pub struct MenuNode {
    pub name: String,
    pub separator: bool,
    pub children: Vec<MenuNode>,
}

/// State of a menu item.
#[derive(Clone, Debug)]
pub struct MenuItemState {
    pub caption: String,
    pub enabled: bool,
    pub visible: bool,
    /// `Some` for check and radio items.
    pub checked: Option<bool>,
    pub radio: bool,
    pub shortcut: Option<u16>,
    pub default: bool,
}

pub(super) struct FormData {
    pub controls: Vec<Control>,
    pub by_name: HashMap<String, usize>,
    pub roots: Vec<usize>,
    pub scale: f32,
    pub text_height: f32,
    pub client: Vec2,
    pub min: Vec2,
    pub caption: String,
    pub menus: HashMap<String, Vec<MenuNode>>,
    pub main_menu: Option<String>,
    pub items: HashMap<String, MenuItemState>,
    pub events: Vec<Event>,
    /// Popup menu to open at the pointer (`TPopupMenu.Popup`).
    pub open_popup: Option<(String, Option<egui::Pos2>)>,
    pub popup_owner: Option<String>,
}

type Painter = Box<dyn FnMut(&mut egui::Ui, Rect)>;

/// A form: controls, menus and their state.
pub struct Form {
    /// Name of the form description (`assets/forms/<name>.json`).
    pub name: String,
    pub obj: FormObj,
    pub(super) data: RefCell<FormData>,
    pub(super) painters: RefCell<HashMap<String, Painter>>,
    pub(super) handlers: RefCell<Vec<(EventKey, Rc<dyn Fn(&Event)>)>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum EventKey {
    Click(String),
    Change(String),
    Menu(String),
    Any,
}

fn anchors_of(o: &FormObj) -> u8 {
    // the port stores corrected anchors in `Tag` (1..15)
    if let Some(t) = o.int("Tag").filter(|t| (1..16).contains(t)) {
        return t as u8;
    }
    match o.set("Anchors") {
        Some(set) => set
            .iter()
            .map(|s| match s.as_str() {
                "akLeft" => AK_LEFT,
                "akTop" => AK_TOP,
                "akRight" => AK_RIGHT,
                "akBottom" => AK_BOTTOM,
                _ => 0,
            })
            .fold(0, |a, b| a | b),
        None => AK_LEFT | AK_TOP,
    }
}

fn align_of(o: &FormObj) -> Align {
    match o.ident("Align") {
        Some("alTop") => Align::Top,
        Some("alBottom") => Align::Bottom,
        Some("alLeft") => Align::Left,
        Some("alRight") => Align::Right,
        Some("alClient") => Align::Client,
        _ => Align::None,
    }
}

fn halign_of(o: &FormObj) -> f32 {
    match o.ident("Alignment") {
        Some("taCenter") => 0.5,
        Some("taRightJustify") => 1.0,
        _ => 0.0,
    }
}

/// VCL colour constant → colour (`None` for the default text colour).
fn vcl_color(id: &str) -> Option<Color32> {
    Some(match id {
        "clRed" => Color32::from_rgb(0xe0, 0x20, 0x20),
        "clMaroon" => Color32::from_rgb(0x80, 0, 0),
        "clGreen" => Color32::from_rgb(0, 0x80, 0),
        "clBlue" | "clHotLight" => Color32::from_rgb(0x20, 0x60, 0xe0),
        "clNavy" => Color32::from_rgb(0, 0, 0x80),
        "clGray" | "clGrayText" => Color32::GRAY,
        _ => return None,
    })
}

fn font_of(o: &FormObj, form_font_height: i64) -> FontStyle {
    let style = o.set("Font.Style").unwrap_or_default();
    let has = |s: &str| style.iter().any(|x| x == s);
    let scale = match o.int("Font.Height") {
        Some(h) if form_font_height != 0 && h != form_font_height => h as f32 / form_font_height as f32,
        _ => 1.0,
    };
    let name = o.str("Font.Name").unwrap_or("");
    FontStyle {
        bold: has("fsBold"),
        italic: has("fsItalic"),
        underline: has("fsUnderline"),
        mono: name.contains("Courier") || name.contains("Consolas") || name.contains("Lucida Console"),
        scale,
        color: o.ident("Font.Color").and_then(vcl_color),
        family: None,
    }
}

/// `DesignSize` of a control (tab sheets have no `Width`/`Height`).
fn design_size(o: &FormObj) -> Option<(i64, i64)> {
    let l = o.props.get("DesignSize")?.get("list")?.as_array()?;
    Some((l.first()?.as_i64()?, l.get(1)?.as_i64()?))
}

struct Builder<'a> {
    controls: Vec<Control>,
    scale: f32,
    form_font_height: i64,
    text_height: i64,
    images: HashMap<String, (Vec<Option<Icon>>, f32)>,
    form_name: &'a str,
    sheet_size: (i64, i64),
}

impl Builder<'_> {
    fn sc(&self, v: i64) -> f32 {
        v as f32 * self.scale
    }

    fn icon(&self, o: &FormObj, list_prop: &str) -> Option<Icon> {
        let (list, size) = self.images.get(o.ident(list_prop)?)?;
        let mut icon = list.get(o.int("ImageIndex")? as usize)?.clone()?;
        icon.size = *size;
        Some(icon)
    }

    fn state(&mut self, o: &FormObj) -> Option<State> {
        let strings = |k: &str| o.strings(k);
        Some(match o.class.as_str() {
            "TLabel" => State::Label {
                halign: halign_of(o),
                wrap: o.bool_or("WordWrap", false),
                vcenter: o.ident("Layout") == Some("tlCenter"),
                focus_control: o.ident("FocusControl").map(String::from),
            },
            "TLinkLabel" => State::Link,
            "TButton" => State::Button { default: o.bool_or("Default", false), cancel: o.bool_or("Cancel", false) },
            "TSpeedButton" => State::Speed {
                toggle: o.int_or("GroupIndex", 0) != 0 || o.bool_or("AllowAllUp", false),
                down: o.bool_or("Down", false),
                flat: o.bool_or("Flat", false),
                icon: self.icon(o, "Images"),
                show_caption: true,
            },
            "TCheckBox" => State::Check { checked: o.bool_or("Checked", false) || o.ident("State") == Some("cbChecked") },
            "TRadioButton" => State::Radio { checked: o.bool_or("Checked", false) },
            "TEdit" | "THotKey" => State::Edit {
                text: TextBuf::plain(o.str("Text").unwrap_or("")),
                password: o.int("PasswordChar").is_some() || o.str("PasswordChar").is_some_and(|s| !s.is_empty()),
                readonly: o.bool_or("ReadOnly", false),
                max_len: o.int("MaxLength").filter(|&m| m > 0).map_or(0, |m| m as usize),
                halign: halign_of(o),
            },
            "TMemo" | "TRichEdit" => State::Memo {
                text: TextBuf::plain(&strings("Lines.Strings").join("\n")),
                readonly: o.bool_or("ReadOnly", false),
                wrap: o.bool_or("WordWrap", true),
            },
            "TComboBox" | "TComboBoxEx" => {
                let items = strings("Items.Strings");
                let index = o.int("ItemIndex").filter(|&i| i >= 0).map(|i| i as usize).filter(|&i| i < items.len());
                let editable = o.class == "TComboBox" && o.ident("Style") != Some("csDropDownList");
                let text = if editable {
                    o.str("Text").unwrap_or("").to_string()
                } else {
                    index.map(|i| items[i].clone()).unwrap_or_default()
                };
                State::Combo { items, index, text: TextBuf::plain(&text), editable }
            }
            "TUpDown" => State::UpDown {
                min: o.int_or("Min", 0),
                max: o.int_or("Max", 100),
                inc: o.int_or("Increment", 1),
                pos: o.int_or("Position", 0),
                associate: o.ident("Associate").map(String::from),
            },
            "TGroupBox" => State::Group,
            "TRadioGroup" => State::RadioGroup {
                items: strings("Items.Strings").iter().map(|s| trl(s)).collect(),
                index: o.int("ItemIndex").filter(|&i| i >= 0).map(|i| i as usize),
                columns: o.int_or("Columns", 1).max(1) as usize,
            },
            "TPanel" | "TScrollBox" => {
                State::Panel { bevel: o.class == "TPanel" && o.ident("BevelOuter") != Some("bvNone") && align_of(o) == Align::None }
            }
            "TTabSheet" => State::Sheet,
            "TPageControl" => State::Pages { active: 0 },
            "TListBox" => {
                let items = strings("Items.Strings");
                let marks = o.bool_or("MultiSelect", false).then(|| vec![false; items.len()]);
                State::ListBox { items, sel: None, marks }
            }
            "TCheckListBox" => {
                let items: Vec<String> = strings("Items.Strings").iter().map(|s| trl(s)).collect();
                let n = items.len();
                State::CheckList { items, checked: vec![false; n], sel: None }
            }
            "TStatusBar" => State::Status { text: o.str("SimpleText").unwrap_or("").to_string() },
            "TTrackBar" => State::Track { min: o.int_or("Min", 0), max: o.int_or("Max", 10), pos: o.int_or("Position", 0) },
            "TProgressBar" => State::Progress { pos: o.int_or("Position", 0), max: o.int_or("Max", 100) },
            "TDateTimePicker" => State::Date { ymd: (2026, 1, 1), checked: false, show_check: o.bool_or("ShowCheckbox", false) },
            "TToolBar" => State::ToolBar,
            "TToolButton" => {
                let style = o.ident("Style").unwrap_or("");
                State::ToolButton {
                    icon: None,
                    separator: style == "tbsSeparator" || style == "tbsDivider",
                    down: o.bool_or("Down", false),
                    toggle: style == "tbsCheck",
                }
            }
            "TSplitter" => State::Splitter,
            "TBevel" => State::Bevel { shape: o.ident("Shape").unwrap_or("bsBox").to_string() },
            "TImage" => State::Image { png: None, version: 0, stretch: o.bool_or("Stretch", false), center: o.bool_or("Center", false) },
            "TListView" | "TStringGrid" | "TDrawGrid" | "TCGauge" => State::Custom,
            _ => return None,
        })
    }

    /// Add `o` and its children. Returns the index of the new control.
    fn add(&mut self, o: &FormObj, parent: Option<usize>) -> Option<usize> {
        let state = self.state(o)?;
        let x = self.sc(o.int_or("Left", 0));
        let y = self.sc(o.int_or("Top", 0));
        let (dw, dh) = if o.class == "TTabSheet" {
            design_size(o).unwrap_or(self.sheet_size)
        } else {
            (o.int_or("Width", 0), o.int_or("Height", 0))
        };
        let (w, h) = (self.sc(dw), self.sc(dh));
        let align = align_of(o);
        let autosize = ((o.class == "TLabel" || o.class == "TLinkLabel") && o.bool_or("AutoSize", true) && !o.bool_or("WordWrap", false) && align == Align::None)
            .then(|| halign_of(o));
        let caption = o.str("Caption").map(trl).unwrap_or_default();
        let idx = self.controls.len();
        self.controls.push(Control {
            name: o.name.clone(),
            class: o.class.clone(),
            parent,
            children: Vec::new(),
            design: Rect::from_min_size(pos2(x, y), vec2(w, h)),
            design_client: vec2(w, h),
            anchors: anchors_of(o),
            align,
            autosize,
            caption,
            hint: o.str("Hint").filter(|h| !h.is_empty()).map(trl).unwrap_or_default(),
            visible: o.bool_or("Visible", true),
            enabled: o.bool_or("Enabled", true),
            font: font_of(o, self.form_font_height),
            popup_menu: o.ident("PopupMenu").map(String::from),
            tab_order: o.int("TabOrder").map(|t| t + 1).unwrap_or(0),
            state,
            rect: Rect::NOTHING,
            focus_request: false,
            has_focus: false,
            selection: None,
            edit_ops: Vec::new(),
        });
        if let Some(p) = parent {
            self.controls[p].children.push(idx);
        }
        if o.class == "TPageControl" {
            // client area of the sheets (`TPageControl.DisplayRect`)
            let sheet = (o.int_or("Width", 200) - 8, o.int_or("Height", 200) - (self.text_height + 10) - 6);
            self.sheet_size = o.children.iter().filter(|c| c.class == "TTabSheet").find_map(design_size).unwrap_or(sheet);
            let active = o.ident("ActivePage").map(String::from);
            for (i, s) in o.children.iter().filter(|c| c.class == "TTabSheet").enumerate() {
                self.add(s, Some(idx));
                if active.as_deref() == Some(s.name.as_str()) {
                    self.controls[idx].state = State::Pages { active: i };
                }
            }
        } else if o.class == "TToolBar" {
            let list = o.ident("Images").map(String::from);
            for c in &o.children {
                if let Some(ci) = self.add(c, Some(idx)) {
                    if let (Some(l), Some(i)) = (&list, c.int("ImageIndex")) {
                        if let Some((icons, size)) = self.images.get(l) {
                            if let State::ToolButton { icon, .. } = &mut self.controls[ci].state {
                                *icon = icons.get(i as usize).cloned().flatten().map(|mut ic| {
                                    ic.size = *size;
                                    ic
                                });
                            }
                        }
                    }
                }
            }
        } else {
            let mut kids: Vec<&FormObj> = o.children.iter().filter(|c| c.class != "TTabSheet").collect();
            // non-windowed controls (no tab order) first: they are below
            kids.sort_by_key(|c| c.int("TabOrder").map(|t| t + 1).unwrap_or(0));
            for c in kids {
                self.add(c, Some(idx));
            }
        }
        // a sheet's design client size comes from DesignSize/sheet size
        if o.class == "TTabSheet" {
            self.controls[idx].design_client = vec2(w, h);
        }
        Some(idx)
    }
}

fn collect_menu(items: &[FormObj], states: &mut HashMap<String, MenuItemState>) -> Vec<MenuNode> {
    items
        .iter()
        .filter(|c| c.class == "TMenuItem")
        .map(|it| {
            let caption = it.str("Caption").unwrap_or("");
            if caption == "-" {
                return MenuNode { name: it.name.clone(), separator: true, children: Vec::new() };
            }
            let checkable = it.bool_or("AutoCheck", false) || it.props.contains_key("Checked") || it.bool_or("RadioItem", false);
            states.insert(
                it.name.clone(),
                MenuItemState {
                    caption: trl(caption),
                    enabled: it.bool_or("Enabled", true),
                    visible: it.bool_or("Visible", true),
                    checked: checkable.then(|| it.bool_or("Checked", false)),
                    radio: it.bool_or("RadioItem", false),
                    shortcut: it.int("ShortCut").map(|s| s as u16),
                    default: it.bool_or("Default", false),
                },
            );
            MenuNode { name: it.name.clone(), separator: false, children: collect_menu(&it.children, states) }
        })
        .collect()
}

impl Form {
    /// Build a form from its description. `scale`: points per design pixel
    /// (see [`super::form_scale`]).
    pub fn new(name: &str, obj: FormObj, scale: f32) -> Form {
        let text_height = obj.int_or("TextHeight", 17);
        let mut b = Builder {
            controls: Vec::new(),
            scale,
            form_font_height: obj.int_or("Font.Height", -14),
            text_height,
            images: HashMap::new(),
            form_name: name,
            sheet_size: (100, 100),
        };
        // image lists: TVirtualImageList → images of the image collections,
        // also lists of other forms ("MainForm.ImageList16")
        let mut lists: Vec<(String, &str, FormObj)> = vec![(String::new(), name, obj.clone())];
        let mut external: Vec<String> = Vec::new();
        obj.walk(&mut |o: &FormObj| {
            for prop in ["Images", "SmallImages", "LargeImages"] {
                if let Some(id) = o.ident(prop).filter(|id| id.contains('.')) {
                    if !external.iter().any(|e| e == id) {
                        external.push(id.to_string());
                    }
                }
            }
        });
        let mut ext_forms = Vec::new();
        for id in &external {
            let (fref, _) = id.split_once('.').unwrap();
            let fname = fref.strip_suffix("Form").or_else(|| fref.strip_suffix("Dlg")).unwrap_or(fref).to_string();
            if let Some(src) = pwtool_app::form::try_load_form(&fname) {
                ext_forms.push((format!("{fref}."), fname, src));
            }
        }
        for (prefix, fname, src) in &ext_forms {
            lists.push((prefix.clone(), fname.as_str(), src.clone()));
        }
        let _ = b.form_name;
        for (prefix, src_form, src) in &lists {
            for c in &src.children {
                if c.class == "TVirtualImageList" {
                    let coll = c.ident("ImageCollection").unwrap_or("").to_string();
                    let size = c.int_or("Width", 16) as f32 * scale;
                    let icons = c
                        .collection("Images")
                        .iter()
                        .map(|i| {
                            let n = i.get("CollectionName").and_then(|v| v.as_str()).unwrap_or("");
                            pwtool_app::icons::form_icon(src_form, &coll, n).map(|_| Icon {
                                form: src_form.to_string(),
                                collection: coll.clone(),
                                name: n.to_string(),
                                size,
                            })
                        })
                        .collect();
                    b.images.insert(format!("{prefix}{}", c.name), (icons, size));
                }
            }
        }

        let mut roots = Vec::new();
        let mut kids: Vec<&FormObj> = obj.children.iter().collect();
        kids.sort_by_key(|c| c.int("TabOrder").map(|t| t + 1).unwrap_or(0));
        for c in kids {
            if let Some(i) = b.add(c, None) {
                roots.push(i);
            }
        }
        let by_name = b.controls.iter().enumerate().map(|(i, c)| (c.name.clone(), i)).collect();

        let cw = obj.int("ClientWidth").or(obj.int("Width")).unwrap_or(400) as f32 * scale;
        let ch = obj.int("ClientHeight").or(obj.int("Height")).unwrap_or(300) as f32 * scale;
        let frame_w = (obj.int_or("Width", 0) as f32 * scale - cw).max(0.0);
        let frame_h = (obj.int_or("Height", 0) as f32 * scale - ch).max(0.0);
        let min = vec2(
            obj.int("Constraints.MinWidth").map_or(0.0, |v| v as f32 * scale - frame_w),
            obj.int("Constraints.MinHeight").map_or(0.0, |v| v as f32 * scale - frame_h),
        );

        let mut items = HashMap::new();
        let mut menus = HashMap::new();
        let mut main_menu = None;
        obj.walk(&mut |o: &FormObj| {
            if o.class == "TMainMenu" || o.class == "TPopupMenu" {
                menus.insert(o.name.clone(), collect_menu(&o.children, &mut items));
            }
        });
        if let Some(m) = obj.ident("Menu") {
            main_menu = Some(m.to_string());
        }
        let caption = obj.str("Caption").map(trl).unwrap_or_default();
        let mut controls = b.controls;
        // up-down buttons show their position in the associated edit
        let assoc: Vec<(String, i64)> = controls
            .iter()
            .filter_map(|c| match &c.state {
                State::UpDown { associate: Some(a), pos, .. } => Some((a.clone(), *pos)),
                _ => None,
            })
            .collect();
        for (a, pos) in assoc {
            if let Some(c) = controls.iter_mut().find(|c| c.name == a) {
                if let State::Edit { text, .. } = &mut c.state {
                    text.set(&pos.to_string());
                }
            }
        }
        let active = obj.ident("ActiveControl").map(String::from);
        if let Some(c) = active.and_then(|a| controls.iter_mut().find(|c| c.name == a)) {
            c.focus_request = true;
        }
        Form {
            name: name.to_string(),
            obj,
            data: RefCell::new(FormData {
                controls,
                by_name,
                roots,
                scale,
                text_height: text_height as f32 * scale,
                client: vec2(cw, ch),
                min,
                caption,
                menus,
                main_menu,
                items,
                events: Vec::new(),
                open_popup: None,
                popup_owner: None,
            }),
            painters: RefCell::new(HashMap::new()),
            handlers: RefCell::new(Vec::new()),
        }
    }

    /// Load and build an embedded form (`assets/forms/<name>.json`).
    pub fn load(name: &str, scale_for: impl FnOnce(&FormObj) -> f32) -> Form {
        let obj = pwtool_app::form::load_form(name);
        let scale = scale_for(&obj);
        Form::new(name, obj, scale)
    }

    // ----------------------------------------------------------- general

    pub fn scale(&self) -> f32 {
        self.data.borrow().scale
    }

    /// Scale a design value to points.
    pub fn sc(&self, v: i64) -> f32 {
        v as f32 * self.scale()
    }

    /// Client size of the form at design time (points).
    pub fn client_size(&self) -> Vec2 {
        self.data.borrow().client
    }

    /// Minimum client size (`Constraints`), points.
    pub fn min_size(&self) -> Vec2 {
        self.data.borrow().min
    }

    pub fn caption(&self) -> String {
        self.data.borrow().caption.clone()
    }

    pub fn set_caption(&self, s: &str) {
        self.data.borrow_mut().caption = s.to_string();
    }

    pub fn has_main_menu(&self) -> bool {
        self.data.borrow().main_menu.is_some()
    }

    pub fn has(&self, name: &str) -> bool {
        self.data.borrow().by_name.contains_key(name)
    }

    /// Names of all controls (in creation order).
    pub fn control_names(&self) -> Vec<String> {
        self.data.borrow().controls.iter().map(|c| c.name.clone()).collect()
    }

    fn with<R>(&self, name: &str, f: impl FnOnce(&mut Control) -> R) -> R {
        let mut d = self.data.borrow_mut();
        let i = *d.by_name.get(name).unwrap_or_else(|| panic!("form {}: no control {name}", self.name));
        f(&mut d.controls[i])
    }

    /// Class of a control (`"TEdit"`…).
    pub fn class(&self, name: &str) -> String {
        self.with(name, |c| c.class.clone())
    }

    /// Rectangle of a control on screen after the last frame.
    pub fn rect(&self, name: &str) -> Rect {
        self.with(name, |c| c.rect)
    }

    /// Move or resize a control (design coordinates, unscaled).
    pub fn set_bounds(&self, name: &str, x: i64, y: i64, w: i64, h: i64) {
        let s = self.scale();
        self.with(name, |c| c.design = Rect::from_min_size(pos2(x as f32 * s, y as f32 * s), vec2(w as f32 * s, h as f32 * s)));
    }

    /// Design rectangle (unscaled design pixels) of a control.
    pub fn bounds(&self, name: &str) -> (i64, i64, i64, i64) {
        let s = self.scale();
        self.with(name, |c| {
            let r = c.design;
            ((r.min.x / s).round() as i64, (r.min.y / s).round() as i64, (r.width() / s).round() as i64, (r.height() / s).round() as i64)
        })
    }

    pub fn visible(&self, name: &str) -> bool {
        self.with(name, |c| c.visible)
    }

    pub fn set_visible(&self, name: &str, on: bool) {
        self.with(name, |c| c.visible = on);
    }

    pub fn enabled(&self, name: &str) -> bool {
        self.with(name, |c| c.enabled)
    }

    pub fn set_enabled(&self, name: &str, on: bool) {
        self.with(name, |c| c.enabled = on);
    }

    /// Caption of a label, button, check box, group box… (with `&`).
    pub fn control_caption(&self, name: &str) -> String {
        self.with(name, |c| c.caption.clone())
    }

    pub fn set_control_caption(&self, name: &str, s: &str) {
        self.with(name, |c| c.caption = s.to_string());
    }

    pub fn hint(&self, name: &str) -> String {
        self.with(name, |c| c.hint.clone())
    }

    pub fn set_hint(&self, name: &str, s: &str) {
        self.with(name, |c| c.hint = s.to_string());
    }

    pub fn set_font_color(&self, name: &str, color: Option<Color32>) {
        self.with(name, |c| c.font.color = color);
    }

    pub fn set_bold(&self, name: &str, bold: bool) {
        self.with(name, |c| c.font.bold = bold);
    }

    /// Give a control the keyboard focus.
    pub fn focus(&self, name: &str) {
        self.with(name, |c| c.focus_request = true);
    }

    pub fn has_focus(&self, name: &str) -> bool {
        self.with(name, |c| c.has_focus)
    }

    // ----------------------------------------------------------- text

    /// Text of an edit, memo, combo box, status bar, label (caption).
    pub fn text(&self, name: &str) -> String {
        self.with(name, |c| match &c.state {
            State::Edit { text, .. } | State::Memo { text, .. } | State::Combo { text, .. } => text.as_str().to_string(),
            State::Status { text } => text.clone(),
            _ => c.caption.clone(),
        })
    }

    /// Text that is wiped after use (passwords).
    pub fn secret_text(&self, name: &str) -> Zeroizing<String> {
        self.with(name, |c| match &c.state {
            State::Edit { text, .. } | State::Memo { text, .. } | State::Combo { text, .. } => Zeroizing::new(text.as_str().to_string()),
            _ => Zeroizing::new(String::new()),
        })
    }

    pub fn set_text(&self, name: &str, s: &str) {
        self.with(name, |c| match &mut c.state {
            State::Edit { text, .. } | State::Memo { text, .. } => text.set(s),
            State::Combo { text, items, index, .. } => {
                text.set(s);
                *index = items.iter().position(|i| i == s);
            }
            State::Status { text } => *text = s.to_string(),
            _ => c.caption = s.to_string(),
        });
    }

    /// Keep the text of an edit in a buffer that is wiped on every change.
    pub fn set_secret(&self, name: &str) {
        self.with(name, |c| {
            if let State::Edit { text, .. } | State::Memo { text, .. } | State::Combo { text, .. } = &mut c.state {
                text.make_secret();
            }
        });
    }

    /// Show or mask the text of an edit (`PasswordChar`).
    pub fn set_password_char(&self, name: &str, masked: bool) {
        self.with(name, |c| {
            if let State::Edit { password, .. } = &mut c.state {
                *password = masked;
            }
        });
    }

    pub fn set_read_only(&self, name: &str, ro: bool) {
        self.with(name, |c| match &mut c.state {
            State::Edit { readonly, .. } | State::Memo { readonly, .. } => *readonly = ro,
            _ => {}
        });
    }

    /// Selected range of an edit or memo (character indices), if not empty.
    pub fn selection(&self, name: &str) -> Option<(usize, usize)> {
        self.with(name, |c| c.selection.filter(|(a, b)| a != b))
    }

    /// Selected text of an edit or memo.
    pub fn selected_text(&self, name: &str) -> Zeroizing<String> {
        let sel = self.selection(name);
        self.with(name, |c| {
            let (State::Edit { text, .. } | State::Memo { text, .. } | State::Combo { text, .. }) = &c.state else {
                return Zeroizing::new(String::new());
            };
            match sel {
                Some((a, b)) => Zeroizing::new(text.as_str().chars().skip(a).take(b - a).collect()),
                None => Zeroizing::new(String::new()),
            }
        })
    }

    pub fn select_all(&self, name: &str) {
        self.with(name, |c| {
            c.edit_ops.push(EditOp::SelectAll);
            c.focus_request = true;
        });
    }

    /// Undo the last change of an edit (not for secret text: it keeps no
    /// history).
    pub fn undo(&self, name: &str) {
        self.with(name, |c| c.edit_ops.push(EditOp::Undo));
    }

    /// Delete the selected text. Returns whether something was deleted.
    pub fn delete_selection(&self, name: &str) -> bool {
        let Some((a, b)) = self.selection(name) else { return false };
        self.with(name, |c| {
            if let State::Edit { text, .. } | State::Memo { text, .. } | State::Combo { text, .. } = &mut c.state {
                let s = text.as_str();
                let (ba, bb) = (byte_of(s, a), byte_of(s, b));
                let mut n = Zeroizing::new(String::with_capacity(s.len()));
                n.push_str(&s[..ba]);
                n.push_str(&s[bb..]);
                text.set(&n);
                c.selection = Some((a, a));
                c.edit_ops.push(EditOp::Cursor(a));
            }
        });
        self.push_event(Event::Change(name.to_string()));
        true
    }

    /// Insert text at the cursor, replacing the selection (paste).
    pub fn insert_text(&self, name: &str, ins: &str) {
        self.delete_selection(name);
        self.with(name, |c| {
            let pos = c.selection.map_or(usize::MAX, |(a, _)| a);
            if let State::Edit { text, .. } | State::Memo { text, .. } | State::Combo { text, .. } = &mut c.state {
                let s = text.as_str();
                let pos = pos.min(s.chars().count());
                let b = byte_of(s, pos);
                let mut n = Zeroizing::new(String::with_capacity(s.len() + ins.len()));
                n.push_str(&s[..b]);
                n.push_str(ins);
                n.push_str(&s[b..]);
                text.set(&n);
                let cur = pos + ins.chars().count();
                c.selection = Some((cur, cur));
                c.edit_ops.push(EditOp::Cursor(cur));
            }
        });
        self.push_event(Event::Change(name.to_string()));
    }

    /// Use a monospaced font for a control.
    /// Font family of a control (`None` = form font).
    pub fn set_font_family(&self, name: &str, family: Option<&'static str>) {
        self.with(name, |c| c.font.family = family);
    }

    pub fn set_mono(&self, name: &str, mono: bool) {
        self.with(name, |c| c.font.mono = mono);
    }

    /// Relative font size of a control.
    pub fn set_font_scale(&self, name: &str, scale: f32) {
        self.with(name, |c| c.font.scale = scale);
    }

    /// Alignment of a label: horizontal (0 left ... 1 right), vertically centred.
    pub fn set_label_align(&self, name: &str, h: f32, v_center: bool) {
        self.with(name, |c| {
            if let State::Label { halign, vcenter, .. } = &mut c.state {
                *halign = h;
                *vcenter = v_center;
            }
            if c.autosize.is_some() {
                c.autosize = Some(h);
            }
        });
    }

    // ----------------------------------------------------------- check state

    /// Check box, radio button, speed/tool button (down), radio group item.
    pub fn checked(&self, name: &str) -> bool {
        self.with(name, |c| match &c.state {
            State::Check { checked } | State::Radio { checked } => *checked,
            State::Speed { down, .. } | State::ToolButton { down, .. } => *down,
            State::Date { checked, .. } => *checked,
            _ => false,
        })
    }

    pub fn set_checked(&self, name: &str, on: bool) {
        // radio buttons of the same parent are exclusive
        let mut d = self.data.borrow_mut();
        let i = *d.by_name.get(name).unwrap_or_else(|| panic!("form {}: no control {name}", self.name));
        let (parent, radio) = (d.controls[i].parent, matches!(d.controls[i].state, State::Radio { .. }));
        if radio && on {
            for c in d.controls.iter_mut().filter(|c| c.parent == parent) {
                if let State::Radio { checked } = &mut c.state {
                    *checked = false;
                }
            }
        }
        match &mut d.controls[i].state {
            State::Check { checked } | State::Radio { checked } => *checked = on,
            State::Speed { down, .. } | State::ToolButton { down, .. } => *down = on,
            State::Date { checked, .. } => *checked = on,
            _ => {}
        }
    }

    // ----------------------------------------------------------- lists

    /// Items of a combo box, list box, check list box, radio group.
    pub fn items(&self, name: &str) -> Vec<String> {
        self.with(name, |c| match &c.state {
            State::Combo { items, .. } | State::ListBox { items, .. } | State::CheckList { items, .. } | State::RadioGroup { items, .. } => {
                items.clone()
            }
            _ => Vec::new(),
        })
    }

    pub fn set_items(&self, name: &str, new: Vec<String>) {
        self.with(name, |c| match &mut c.state {
            State::Combo { items, index, text, editable } => {
                *items = new;
                *index = items.iter().position(|i| i == text.as_str());
                if !*editable && index.is_none() {
                    text.set("");
                }
            }
            State::ListBox { items, sel, marks } => {
                *items = new;
                *sel = sel.filter(|&s| s < items.len());
                if let Some(m) = marks {
                    *m = vec![false; items.len()];
                }
            }
            State::CheckList { items, checked, sel } => {
                checked.resize(new.len(), false);
                *items = new;
                *sel = sel.filter(|&s| s < items.len());
            }
            State::RadioGroup { items, index, .. } => {
                *items = new;
                *index = index.filter(|&s| s < items.len());
            }
            _ => {}
        });
    }

    /// Selected item (`ItemIndex`), also the active page of a page control.
    pub fn item_index(&self, name: &str) -> Option<usize> {
        self.with(name, |c| match &c.state {
            State::Combo { index, .. } | State::RadioGroup { index, .. } => *index,
            State::ListBox { sel, .. } | State::CheckList { sel, .. } => *sel,
            State::Pages { active } => Some(*active),
            _ => None,
        })
    }

    pub fn set_item_index(&self, name: &str, i: Option<usize>) {
        self.with(name, |c| match &mut c.state {
            State::Combo { index, items, text, .. } => {
                *index = i.filter(|&i| i < items.len());
                text.set(index.map(|i| items[i].as_str()).unwrap_or(""));
            }
            State::RadioGroup { index, items, .. } => *index = i.filter(|&i| i < items.len()),
            State::ListBox { sel, items, marks } => {
                *sel = i.filter(|&i| i < items.len());
                if let Some(m) = marks {
                    m.iter_mut().for_each(|x| *x = false);
                    if let Some(s) = *sel {
                        m[s] = true;
                    }
                }
            }
            State::CheckList { sel, items, .. } => *sel = i.filter(|&i| i < items.len()),
            State::Pages { active } => *active = i.unwrap_or(0),
            _ => {}
        });
    }

    /// Selected items of a list box (`Selected[]`; with single selection
    /// the item index).
    pub fn list_selected(&self, name: &str) -> Vec<usize> {
        self.with(name, |c| match &c.state {
            State::ListBox { marks: Some(m), .. } => m.iter().enumerate().filter(|(_, x)| **x).map(|(i, _)| i).collect(),
            State::ListBox { sel, .. } => sel.iter().copied().collect(),
            _ => Vec::new(),
        })
    }

    /// Clear the selection of a list box.
    pub fn clear_selection(&self, name: &str) {
        self.with(name, |c| {
            if let State::ListBox { sel, marks, .. } = &mut c.state {
                *sel = None;
                if let Some(m) = marks {
                    m.iter_mut().for_each(|x| *x = false);
                }
            }
        });
    }

    /// Check marks of a check list box.
    pub fn list_checked(&self, name: &str) -> Vec<bool> {
        self.with(name, |c| match &c.state {
            State::CheckList { checked, .. } => checked.clone(),
            _ => Vec::new(),
        })
    }

    pub fn set_list_checked(&self, name: &str, i: usize, on: bool) {
        self.with(name, |c| {
            if let State::CheckList { checked, .. } = &mut c.state {
                if let Some(x) = checked.get_mut(i) {
                    *x = on;
                }
            }
        });
    }

    /// Active page of a page control by the name of the sheet.
    pub fn set_active_page(&self, pages: &str, sheet: &str) {
        let idx = {
            let d = self.data.borrow();
            let p = d.by_name[pages];
            d.controls[p].children.iter().position(|&c| d.controls[c].name == sheet)
        };
        self.set_item_index(pages, idx);
    }

    // ----------------------------------------------------------- numbers

    /// Position of an up-down button (read from the associated edit), track
    /// bar, progress bar.
    pub fn position(&self, name: &str) -> i64 {
        let (assoc, min, max, pos) = self.with(name, |c| match &c.state {
            State::UpDown { min, max, pos, associate, .. } => (associate.clone(), *min, *max, *pos),
            State::Track { pos, .. } | State::Progress { pos, .. } => (None, i64::MIN, i64::MAX, *pos),
            _ => (None, 0, 0, 0),
        });
        if let Some(a) = assoc {
            if let Ok(v) = self.text(&a).trim().parse::<i64>() {
                return v.clamp(min, max);
            }
        }
        pos
    }

    pub fn set_position(&self, name: &str, v: i64) {
        let assoc = self.with(name, |c| match &mut c.state {
            State::UpDown { min, max, pos, associate, .. } => {
                *pos = v.clamp(*min, *max);
                associate.clone().map(|a| (a, *pos))
            }
            State::Track { min, max, pos } => {
                *pos = v.clamp(*min, *max);
                None
            }
            State::Progress { pos, .. } => {
                *pos = v;
                None
            }
            _ => None,
        });
        if let Some((a, p)) = assoc {
            self.set_text(&a, &p.to_string());
        }
    }

    /// Range of an up-down button, track bar or progress bar (`Min`, `Max`).
    pub fn set_range(&self, name: &str, lo: i64, hi: i64) {
        self.with(name, |c| match &mut c.state {
            State::UpDown { min, max, .. } | State::Track { min, max, .. } => {
                *min = lo;
                *max = hi;
            }
            State::Progress { max, .. } => *max = hi,
            _ => {}
        });
    }

    /// Date of a date picker: (year, month, day).
    pub fn date(&self, name: &str) -> (i32, u32, u32) {
        self.with(name, |c| match &c.state {
            State::Date { ymd, .. } => *ymd,
            _ => (2000, 1, 1),
        })
    }

    pub fn set_date(&self, name: &str, y: i32, m: u32, d: u32) {
        self.with(name, |c| {
            if let State::Date { ymd, .. } = &mut c.state {
                *ymd = (y, m, d);
            }
        });
    }

    // ----------------------------------------------------------- images, drawing

    /// Picture of a `TImage` (PNG).
    pub fn set_image(&self, name: &str, png: Option<Vec<u8>>) {
        self.with(name, |c| {
            if let State::Image { png: p, version, .. } = &mut c.state {
                *p = png;
                *version += 1;
            }
        });
    }

    /// Draw a control with `f` (list views, grids, gauges and any other
    /// control the logic draws itself). `f` gets a `Ui` limited to the
    /// control and its rectangle.
    pub fn set_painter(&self, name: &str, f: impl FnMut(&mut egui::Ui, Rect) + 'static) {
        self.painters.borrow_mut().insert(name.to_string(), Box::new(f));
    }

    // ----------------------------------------------------------- menus

    fn item<R>(&self, name: &str, f: impl FnOnce(&mut MenuItemState) -> R) -> R {
        let mut d = self.data.borrow_mut();
        let it = d.items.get_mut(name).unwrap_or_else(|| panic!("form {}: no menu item {name}", self.name));
        f(it)
    }

    pub fn has_menu_item(&self, name: &str) -> bool {
        self.data.borrow().items.contains_key(name)
    }

    pub fn menu_enabled(&self, name: &str) -> bool {
        self.item(name, |i| i.enabled)
    }

    pub fn set_menu_enabled(&self, name: &str, on: bool) {
        self.item(name, |i| i.enabled = on);
    }

    pub fn set_menu_visible(&self, name: &str, on: bool) {
        self.item(name, |i| i.visible = on);
    }

    pub fn menu_checked(&self, name: &str) -> bool {
        self.item(name, |i| i.checked.unwrap_or(false))
    }

    /// Check or uncheck a menu item (makes it a check item). Radio items of
    /// the same group are exclusive.
    pub fn set_menu_checked(&self, name: &str, on: bool) {
        self.item(name, |i| i.checked = Some(on));
    }

    pub fn set_menu_caption(&self, name: &str, caption: &str) {
        self.item(name, |i| i.caption = caption.to_string());
    }

    pub fn menu_caption(&self, name: &str) -> String {
        self.item(name, |i| i.caption.clone())
    }

    /// Replace the sub-items of menu item `parent` with dynamic items
    /// (name, caption) placed before the items of the form description;
    /// they produce [`Event::Menu`] with the name `"dyn:<name>"`.
    pub fn set_dynamic_items(&self, parent: &str, new: &[(String, String, Option<u16>)]) {
        let mut d = self.data.borrow_mut();
        let FormData { menus, items, .. } = &mut *d;
        fn find<'a>(nodes: &'a mut [MenuNode], name: &str) -> Option<&'a mut MenuNode> {
            for n in nodes {
                if n.name == name {
                    return Some(n);
                }
                if let Some(x) = find(&mut n.children, name) {
                    return Some(x);
                }
            }
            None
        }
        for tree in menus.values_mut() {
            if let Some(node) = find(tree, parent) {
                node.children.retain(|c| !c.name.starts_with("dyn:"));
                let mut dynamic: Vec<MenuNode> = Vec::new();
                for (n, caption, shortcut) in new {
                    let key = format!("dyn:{n}");
                    items.insert(
                        key.clone(),
                        MenuItemState {
                            caption: caption.clone(),
                            enabled: true,
                            visible: true,
                            checked: None,
                            radio: false,
                            shortcut: *shortcut,
                            default: false,
                        },
                    );
                    dynamic.push(MenuNode { name: key, separator: false, children: Vec::new() });
                }
                if !dynamic.is_empty() && !node.children.is_empty() {
                    dynamic.push(MenuNode { name: "dyn:-".into(), separator: true, children: Vec::new() });
                }
                node.children.splice(0..0, dynamic);
            }
        }
    }

    /// Attach a popup menu to a control (right click), like `PopupMenu`.
    pub fn set_popup_menu(&self, name: &str, menu: &str) {
        self.with(name, |c| c.popup_menu = Some(menu.to_string()));
    }

    /// Open a popup menu at `pos` (or the pointer).
    pub fn popup_menu(&self, menu: &str, pos: Option<egui::Pos2>) {
        self.data.borrow_mut().open_popup = Some((menu.to_string(), pos));
    }

    /// Control whose context menu was opened last (`PopupComponent`).
    pub fn popup_owner(&self) -> Option<String> {
        self.data.borrow().popup_owner.clone()
    }

    // ----------------------------------------------------------- events

    /// Take the events recorded since the last call.
    pub fn take_events(&self) -> Vec<Event> {
        std::mem::take(&mut self.data.borrow_mut().events)
    }

    /// Record an event as if the user had caused it (e.g. `Click` of a
    /// button from the logic or test automation).
    pub fn push_event(&self, e: Event) {
        self.data.borrow_mut().events.push(e);
    }

    pub fn on_click(&self, name: &str, f: impl Fn(&Event) + 'static) {
        self.handlers.borrow_mut().push((EventKey::Click(name.to_string()), Rc::new(f)));
    }

    pub fn on_change(&self, name: &str, f: impl Fn(&Event) + 'static) {
        self.handlers.borrow_mut().push((EventKey::Change(name.to_string()), Rc::new(f)));
    }

    pub fn on_menu(&self, name: &str, f: impl Fn(&Event) + 'static) {
        self.handlers.borrow_mut().push((EventKey::Menu(name.to_string()), Rc::new(f)));
    }

    /// Handler for every event not handled by a specific handler.
    pub fn on_any(&self, f: impl Fn(&Event) + 'static) {
        self.handlers.borrow_mut().push((EventKey::Any, Rc::new(f)));
    }

    pub fn has_handlers(&self) -> bool {
        !self.handlers.borrow().is_empty()
    }

    /// Call the handlers of the recorded events. Returns the events without
    /// a handler.
    pub fn dispatch(&self) -> Vec<Event> {
        let mut rest = Vec::new();
        for e in self.take_events() {
            let key = match &e {
                Event::Click(n) => EventKey::Click(n.clone()),
                Event::Change(n) => EventKey::Change(n.clone()),
                Event::Menu(n) => EventKey::Menu(n.clone()),
                _ => EventKey::Any,
            };
            let hs: Vec<Rc<dyn Fn(&Event)>> =
                self.handlers.borrow().iter().filter(|(k, _)| *k == key).map(|(_, h)| h.clone()).collect();
            let hs = if hs.is_empty() {
                self.handlers.borrow().iter().filter(|(k, _)| *k == EventKey::Any).map(|(_, h)| h.clone()).collect()
            } else {
                hs
            };
            if hs.is_empty() {
                rest.push(e);
            } else {
                for h in hs {
                    h(&e);
                }
            }
        }
        rest
    }
}
