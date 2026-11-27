//! UI Automation fragments for the active, drawn recovery confirmation.

use std::rc::Rc;

use nsh_core::NodeId;
use windows::{
    Win32::{
        Foundation::HWND,
        System::{Com::SAFEARRAY, Variant::VARIANT},
        UI::{Accessibility::*, Input::KeyboardAndMouse::GetFocus, WindowsAndMessaging::IsChild},
    },
    core::{IUnknown, IUnknownImpl, Result, implement},
};

use super::{
    Root, Var, client_origin, none, not_available, not_enabled, runtime_array, scale_in,
    surface_of, window,
};
use crate::{
    surface::Surface,
    system::{RecoveredDraftChoice, RecoveryAccessibility, recovery_accessibility},
    view::NshView,
};

pub(super) fn owns_keyboard(hwnd: HWND) -> bool {
    if hwnd.is_invalid() {
        return false;
    }
    // SAFETY: read-only queries on the UI thread that owns the dialog.
    unsafe {
        let focused = GetFocus();
        focused == hwnd || (!focused.is_invalid() && IsChild(hwnd, focused).as_bool())
    }
}

pub(super) fn client_size(root: &NshView) -> (f32, f32) {
    let size = root.state().frame.size;
    (size.width.0, size.height.0)
}

/// Holds only instance identity. A provider kept by a UIA client cannot
/// retain the dialog, its backdrop, or a capability to answer it later.
#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IInvokeProvider
)]
pub(super) struct Node {
    key: u64,
    window: NodeId,
    serial: u64,
    button: Option<usize>,
}

impl Node {
    pub(super) fn new(key: u64, window: NodeId, serial: u64, button: Option<usize>) -> Self {
        Self {
            key,
            window,
            serial,
            button,
        }
    }
}

impl Node_Impl {
    fn resolve(&self) -> Result<(Rc<Surface>, RecoveryAccessibility)> {
        let surface = surface_of(self.key)?;
        if self.window != nsh_core::ROOT_ID
            && crate::app_window::presentation(&surface, self.window, false).is_none()
        {
            return Err(not_available());
        }
        let dialog = recovery_accessibility(&surface, self.window)
            .filter(|dialog| {
                dialog.serial() == self.serial
                    && self.button.is_none_or(|i| i < dialog.labels().len())
            })
            .ok_or_else(not_available)?;
        Ok((surface, dialog))
    }

    fn node(&self, button: Option<usize>) -> Node {
        Node::new(self.key, self.window, self.serial, button)
    }

    fn root(&self) -> Root {
        Root {
            key: self.key,
            window: self.window,
        }
    }
}

impl IRawElementProviderSimple_Impl for Node_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        self.resolve()?;
        if self.button.is_some() && pattern == UIA_InvokePatternId {
            Ok(self.to_object().to_interface())
        } else {
            none()
        }
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let (surface, dialog) = self.resolve()?;
        let button = self.button;
        let property = match property {
            UIA_ControlTypePropertyId => Var::i32(if button.is_some() {
                UIA_ButtonControlTypeId.0
            } else {
                UIA_PaneControlTypeId.0
            }),
            UIA_IsDialogPropertyId => Var::bool(button.is_none()),
            UIA_NamePropertyId => Var::opt_str(match button {
                Some(i) => dialog.labels().get(i).map(String::as_str),
                None => dialog.title(),
            }),
            UIA_AutomationIdPropertyId => Var::opt_str(match button {
                Some(i) => match dialog.choice(i) {
                    Some(RecoveredDraftChoice::Continue) => Some("texzi.recovery.continue"),
                    Some(RecoveredDraftChoice::Discard) => Some("texzi.recovery.discard"),
                    Some(RecoveredDraftChoice::SaveAs) => Some("texzi.recovery.save-as"),
                    None => None,
                },
                None => Some("texzi.recovery.confirm"),
            }),
            UIA_FullDescriptionPropertyId if button.is_none() => Var::opt_str(dialog.description()),
            UIA_LocalizedControlTypePropertyId if button.is_none() => Var::str("dialog"),
            UIA_IsKeyboardFocusablePropertyId => Var::bool(button.is_some()),
            UIA_HasKeyboardFocusPropertyId => Var::bool(
                button.is_some()
                    && button == dialog.focus()
                    && window(&surface, self.window).is_some_and(|(_, hwnd)| owns_keyboard(hwnd)),
            ),
            UIA_IsEnabledPropertyId => Var::bool(dialog.enabled()),
            UIA_IsControlElementPropertyId | UIA_IsContentElementPropertyId => Var::bool(true),
            UIA_FrameworkIdPropertyId | UIA_ProviderDescriptionPropertyId => Var::str("nsh"),
            _ => Var::empty(),
        };
        Ok(property.into_raw())
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        none()
    }
}

impl IRawElementProviderFragment_Impl for Node_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let (_, dialog) = self.resolve()?;
        let button = match (self.button, direction) {
            (None, NavigateDirection_Parent) => return Ok(self.root().into()),
            (Some(_), NavigateDirection_Parent) => return Ok(self.node(None).into()),
            (None, NavigateDirection_FirstChild) if !dialog.labels().is_empty() => Some(0),
            (None, NavigateDirection_LastChild) => dialog.labels().len().checked_sub(1),
            (Some(i), NavigateDirection_NextSibling) => {
                i.checked_add(1).filter(|&n| n < dialog.labels().len())
            }
            (Some(i), NavigateDirection_PreviousSibling) => i.checked_sub(1),
            _ => None,
        };
        button.map_or_else(none, |index| Ok(self.node(Some(index)).into()))
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        self.resolve()?;
        runtime_array(&[
            UiaAppendRuntimeId as i32,
            self.serial as i32,
            (self.serial >> 32) as i32,
            self.button.map_or(-1, |i| i as i32),
        ])
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let (surface, dialog) = self.resolve()?;
        let (root, hwnd) = window(&surface, self.window).ok_or_else(not_available)?;
        let bounds = dialog
            .bounds(client_size(&root), self.button)
            .ok_or_else(not_available)?;
        let scale = scale_in(&surface, self.window, hwnd) as f64;
        let (ox, oy) = client_origin(hwnd);
        Ok(UiaRect {
            left: ox + f64::from(bounds.origin.x.0) * scale,
            top: oy + f64::from(bounds.origin.y.0) * scale,
            width: f64::from(bounds.size.width.0) * scale,
            height: f64::from(bounds.size.height.0) * scale,
        })
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        let (_, dialog) = self.resolve()?;
        let index = self.button.ok_or_else(super::not_supported)?;
        dialog
            .focus_button(index)
            .then_some(())
            .ok_or_else(not_enabled)
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        self.resolve()?;
        Ok(self.root().into())
    }
}

impl IInvokeProvider_Impl for Node_Impl {
    fn Invoke(&self) -> Result<()> {
        let (_, dialog) = self.resolve()?;
        let index = self.button.ok_or_else(super::not_supported)?;
        if !dialog.invoke(index) {
            return Err(not_enabled());
        }
        let provider: IRawElementProviderSimple = self.node(Some(index)).into();
        // SAFETY: the provider lives through the event call.
        let _ = unsafe { UiaRaiseAutomationEvent(&provider, UIA_Invoke_InvokedEventId) };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::uia::{Element, Var};
    use crate::system::show_recovery_for_access_test;
    use nsh_core::{EventKind, MountItem, ROOT_ID, ViewKind};
    use nsh_engine::Environment;
    use windows::{
        Win32::System::Variant::VT_BSTR,
        core::{Interface, w},
    };

    fn body(surface: &Rc<Surface>) -> IRawElementProviderSimple {
        for item in [
            MountItem::Create {
                id: 4,
                kind: ViewKind::Input,
            },
            MountItem::Insert {
                parent: ROOT_ID,
                child: 4,
                before: None,
            },
        ] {
            surface.mounter.borrow_mut().apply(surface, &item);
        }
        let view = surface.view(4).expect("background input");
        let mut state = view.state_mut();
        state.props.accessibility.label = Some("Background input".into());
        state.events.set(EventKind::Press, true);
        drop(state);
        Element {
            key: surface.key(),
            id: 4,
        }
        .into()
    }

    fn property(provider: &IRawElementProviderSimple, id: UIA_PROPERTY_ID) -> String {
        // SAFETY: the test holds the real server-side provider throughout.
        let value = Var(unsafe { provider.GetPropertyValue(id) }.expect("text property"));
        // SAFETY: the discriminant proves this is the BSTR member; Var owns it.
        unsafe {
            let inner = &*value.0.Anonymous.Anonymous;
            assert_eq!(inner.vt, VT_BSTR);
            inner.Anonymous.bstrVal.to_string()
        }
    }

    fn child(
        provider: &IRawElementProviderFragment,
        direction: NavigateDirection,
    ) -> IRawElementProviderFragment {
        // SAFETY: the test holds both sides of the in-process provider call.
        unsafe { provider.Navigate(direction) }.expect("provider child")
    }

    fn name(fragment: &IRawElementProviderFragment) -> String {
        let provider = fragment.cast::<IRawElementProviderSimple>().unwrap();
        property(&provider, UIA_NamePropertyId)
    }

    #[test]
    fn recovery_providers_replace_only_the_current_tree_and_expire_on_answer() {
        let environment = Environment {
            reduce_motion: true,
            ..Environment::default()
        };
        // Neither Surface has a visible HWND. The fixture binds internally to
        // the test-only root so the actual provider methods can be called.
        let surface = Surface::new(environment);
        let other = Surface::new(environment);
        let background = body(&surface);
        let other_background = body(&other);
        let root: IRawElementProviderFragment = Root {
            key: surface.key(),
            window: ROOT_ID,
        }
        .into();
        assert_eq!(
            name(&child(&root, NavigateDirection_FirstChild)),
            "Background input"
        );

        let (serial, mut reply) = show_recovery_for_access_test(&surface, false);
        let pane = child(&root, NavigateDirection_FirstChild);
        assert_eq!(name(&pane), "Recovered draft");
        let keep = child(&pane, NavigateDirection_FirstChild);
        let discard = child(&keep, NavigateDirection_NextSibling);
        let save = child(&discard, NavigateDirection_NextSibling);
        assert_eq!(
            [name(&keep), name(&discard), name(&save)],
            ["Keep editing", "Discard", "Save As"]
        );
        assert_eq!(
            recovery_accessibility(&surface, ROOT_ID).unwrap().focus(),
            Some(0)
        );
        assert_eq!(
            property(&other_background, UIA_NamePropertyId),
            "Background input"
        );

        let wrong_window: IRawElementProviderSimple =
            Node::new(surface.key(), 99, serial, Some(0)).into();
        // SAFETY: each call is to a live provider object in this apartment.
        unsafe {
            assert_eq!(
                wrong_window
                    .GetPropertyValue(UIA_NamePropertyId)
                    .err()
                    .unwrap()
                    .code()
                    .0,
                UIA_E_ELEMENTNOTAVAILABLE as i32,
            );
            assert_eq!(
                background
                    .cast::<IInvokeProvider>()
                    .unwrap()
                    .Invoke()
                    .unwrap_err()
                    .code()
                    .0,
                UIA_E_ELEMENTNOTAVAILABLE as i32,
            );
            assert_eq!(
                background
                    .cast::<IValueProvider>()
                    .unwrap()
                    .SetValue(w!("blocked"))
                    .unwrap_err()
                    .code()
                    .0,
                UIA_E_ELEMENTNOTAVAILABLE as i32,
            );
            assert_eq!(
                background
                    .cast::<IRawElementProviderFragment>()
                    .unwrap()
                    .SetFocus()
                    .unwrap_err()
                    .code()
                    .0,
                UIA_E_ELEMENTNOTAVAILABLE as i32,
            );
            assert_eq!(
                background
                    .cast::<ISelectionItemProvider>()
                    .unwrap()
                    .Select()
                    .unwrap_err()
                    .code()
                    .0,
                UIA_E_ELEMENTNOTAVAILABLE as i32,
            );
            save.SetFocus().unwrap();
        }
        assert_eq!(
            recovery_accessibility(&surface, ROOT_ID).unwrap().focus(),
            Some(2)
        );

        // A reader may explicitly activate the safe action without moving
        // selection first. Use the same queued answer path as a click.
        let invoke = keep.cast::<IInvokeProvider>().unwrap();
        unsafe { invoke.Invoke() }.unwrap();
        assert_eq!(
            name(&child(&root, NavigateDirection_FirstChild)),
            "Recovered draft"
        );
        surface.ui_thread().dispatch_pending_calls();
        assert_eq!(reply.try_recv().unwrap(), RecoveredDraftChoice::Continue);
        assert_eq!(
            name(&child(&root, NavigateDirection_FirstChild)),
            "Background input"
        );

        let (new_serial, _reply) = show_recovery_for_access_test(&surface, true);
        assert_ne!(serial, new_serial);
        assert_eq!(
            recovery_accessibility(&surface, ROOT_ID).unwrap().focus(),
            Some(2)
        );
        assert_eq!(
            unsafe { invoke.Invoke() }.unwrap_err().code().0,
            UIA_E_ELEMENTNOTAVAILABLE as i32,
        );
        crate::system::clear(&surface);
    }
}
