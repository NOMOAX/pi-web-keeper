//! 开始菜单／任务栏图标右键菜单里的「任务」那一段（Jump List Tasks）。
//!
//! 这一段不是程序自己画的：它由 shell 从 COM 读出来，由 Windows 画。
//! 所以它和托盘图标的右键菜单是两套完全不同的东西 ——
//! 托盘菜单是我们自己画的，跳转列表是登记给 shell 的。
//!
//! 硬性前提：进程必须有显式 AppUserModelID，而且要在创建任何窗口之前设好。
//! 两个连带后果：
//!   - 任务栏图标的身份会从「按路径派生」变成这个显式 ID，已经固定过的旧项
//!     不会带上跳转列表，要重新固定一次；
//!   - 别跟 `settings.json` 里的 `AppUserModelId` 弄混 —— 那个是**被托管的 PWA** 的身份。
//!
//! 每次启动都要重写一遍：Windows 会因为超限或用户清空「最近使用」把它丢掉。

use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, StructuredStorage::PROPVARIANT, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY};
use windows::Win32::UI::Shell::{
    ICustomDestinationList, IObjectArray, IObjectCollection, IShellLinkW, SetCurrentProcessExplicitAppUserModelID,
    CLSID_DestinationList, CLSID_EnumerableObjectCollection, CLSID_ShellLink,
};
use windows::Win32::Foundation::GUID;

/// 启动器自己的 AppUserModelID。
///
/// 与 `settings.json` 里那个（被托管的 PWA）没有任何关系。
pub const APP_USER_MODEL_ID: &str = "PiWebKeeper.Launcher";

/// 任务那一行的文字。
///
/// 直接写中文而不是让 Windows 本地化：这个程序只有中文界面，
/// 传 `"Tasks"` 反而会在中文系统上显示成英文。
const TASK_TITLE: &str = "以普通方式启动";

/// 任务那一组的标题。
const CATEGORY_TITLE: &str = "任务";

/// 任务启动时带的参数：显示界面、不自动启动服务。
const TASK_ARGUMENT: &str = crate::command::NO_AUTOSTART_FLAG;

/// PKEY_Title：{F29F85E0-4FF9-1068-AB91-08002B27B3D9}, 2。
///
/// 任务那一行显示什么文字，不看 `SetDescription`，看 `IShellLink` 属性存储里的这一条。
const PKEY_TITLE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xF29F_85E0_4FF9_1068_AB91_0800_2B27_B3D9),
    pid:   2,
};

/// 登记跳转列表。失败不致命：没有它，程序照常工作，只是开始菜单里少一行。
pub fn install() -> Result<(), String> {
    // SAFETY: 单线程 COM 初始化；已经初始化过会返回 S_FALSE，同样可用
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    if initialized.is_err() {
        // RPC_E_CHANGED_MODE 说明这个线程已经用别的模型初始化过 COM。
        // 跳转列表用不到代理，继续往下走。
        let code = initialized.0;
        const RPC_E_CHANGED_MODE: i32 = 0x8001_0106u32 as i32;

        if code != RPC_E_CHANGED_MODE {
            return Err(format!("COM 初始化失败：0x{code:08X}"));
        }
    }

    let executable = std::env::current_exe()
        .map_err(|error| format!("拿不到本程序路径：{error}"))?
        .to_string_lossy()
        .into_owned();

    // SAFETY: 字符串在调用期间有效
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(APP_USER_MODEL_ID))
            .map_err(|error| format!("设置 AppUserModelID 失败：{error}"))?;

        let destinations: ICustomDestinationList =
            CoCreateInstance(&CLSID_DestinationList, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format!("创建跳转列表失败：{error}"))?;

        let mut max_slots = 0u32;

        // 返回值是「被移除的项」，这里用不上，但必须接住
        let _removed: IObjectArray = destinations
            .BeginList(&mut max_slots)
            .map_err(|error| format!("开始写跳转列表失败：{error}"))?;

        let link: IShellLinkW = CoCreateInstance(&CLSID_ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|error| format!("创建快捷方式失败：{error}"))?;

        let path = HSTRING::from(executable.as_str());

        link.SetPath(&path)
            .map_err(|error| format!("设置任务路径失败：{error}"))?;

        link.SetArguments(&HSTRING::from(TASK_ARGUMENT))
            .map_err(|error| format!("设置任务参数失败：{error}"))?;

        link.SetIconLocation(&path, 0)
            .map_err(|error| format!("设置任务图标失败：{error}"))?;

        let store: IPropertyStore = link
            .cast()
            .map_err(|error| format!("取快捷方式属性存储失败：{error}"))?;

        let title = PROPVARIANT::from(&HSTRING::from(TASK_TITLE));

        store
            .SetValue(&PKEY_TITLE, &title)
            .map_err(|error| format!("设置任务标题失败：{error}"))?;

        store
            .Commit()
            .map_err(|error| format!("提交任务标题失败：{error}"))?;

        let collection: IObjectCollection =
            CoCreateInstance(&CLSID_EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format!("创建任务集合失败：{error}"))?;

        collection
            .AddObject(&link)
            .map_err(|error| format!("把任务加进集合失败：{error}"))?;

        let array: IObjectArray = collection
            .cast()
            .map_err(|error| format!("取任务数组失败：{error}"))?;

        destinations
            .AppendCategory(&HSTRING::from(CATEGORY_TITLE), &array)
            .map_err(|error| format!("追加任务分组失败：{error}"))?;

        destinations
            .CommitList()
            .map_err(|error| format!("提交跳转列表失败：{error}"))?;
    }

    Ok(())
}
