//! Abstract Human Interface Device Interfaces
use core::marker::PhantomData;
use frunk::{HCons, HNil, ToRef};
use packed_struct::prelude::*;
use usb_device::bus::{InterfaceNumber, StringIndex, UsbBus, UsbBusAllocator};
use usb_device::class_prelude::DescriptorWriter;

use crate::hid_class::descriptor::{
    DescriptorType, HidProtocol, COUNTRY_CODE_NOT_SUPPORTED, SPEC_VERSION_1_11,
};

use self::raw::RawInterface;

pub mod managed;
pub mod raw;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PackedStruct)]
#[packed_struct(endian = "lsb", size_bytes = 7)]
pub struct HidDescriptorBody {
    bcd_hid: u16,
    country_code: u8,
    num_descriptors: u8,
    #[packed_field(ty = "enum", size_bytes = "1")]
    descriptor_type: DescriptorType,
    descriptor_length: u16,
}

pub trait UsbAllocatable<'a, B: UsbBus> {
    type Allocated;
    fn allocate(self, usb_alloc: &'a UsbBusAllocator<B>) -> Self::Allocated;
}

impl<'a, B: UsbBus + 'a> UsbAllocatable<'a, B> for HNil {
    type Allocated = HNil;

    fn allocate(self, _: &'a UsbBusAllocator<B>) -> Self::Allocated {
        self
    }
}

impl<'a, B, C, Tail> UsbAllocatable<'a, B> for HCons<C, Tail>
where
    B: UsbBus + 'a,
    C: UsbAllocatable<'a, B>,
    Tail: UsbAllocatable<'a, B>,
{
    type Allocated = HCons<C::Allocated, Tail::Allocated>;

    fn allocate(self, usb_alloc: &'a UsbBusAllocator<B>) -> Self::Allocated {
        HCons {
            head: self.head.allocate(usb_alloc),
            tail: self.tail.allocate(usb_alloc),
        }
    }
}

pub trait InterfaceClass<'a, B: UsbBus> {
    fn report_descriptor(&self) -> &'_ [u8];
    fn id(&self) -> InterfaceNumber;
    fn write_descriptors(&self, writer: &mut DescriptorWriter) -> usb_device::Result<()>;
    fn get_string(&self, index: StringIndex, lang_id: u16) -> Option<&'_ str>;
    fn reset(&mut self);
    fn set_report(&mut self, data: &[u8]) -> usb_device::Result<()>;
    fn get_report(&mut self, data: &mut [u8]) -> usb_device::Result<usize>;
    fn get_report_ack(&mut self) -> usb_device::Result<()>;
    fn set_idle(&mut self, report_id: u8, value: u8);
    fn get_idle(&self, report_id: u8) -> u8;
    fn set_protocol(&mut self, protocol: HidProtocol);
    fn get_protocol(&self) -> HidProtocol;
    fn hid_descriptor_body(&self) -> [u8; 7] {
        let descriptor_len = u16::try_from(self.report_descriptor().len())
            .expect("Report descriptor too long, must be < u16::MAX");

        HidDescriptorBody {
            bcd_hid: SPEC_VERSION_1_11,
            country_code: COUNTRY_CODE_NOT_SUPPORTED,
            num_descriptors: 1,
            descriptor_type: DescriptorType::Report,
            descriptor_length: descriptor_len,
        }
        .pack()
        .map_err(drop) // Avoid pulling all the core::fmt code into final binary
        .expect("Failed to pack HidDescriptor")
    }
    fn interface(&self) -> &RawInterface<'a, B>;
}

pub trait InterfaceHList<'a, B>: ToRef<'a> {
    fn get_id_mut(&mut self, id: u8) -> Option<&mut dyn InterfaceClass<'a, B>>;
    fn get_id(&self, id: u8) -> Option<&dyn InterfaceClass<'a, B>>;
    fn reset(&mut self);
    fn write_descriptors(&self, writer: &mut DescriptorWriter) -> usb_device::Result<()>;
    fn get_string(&self, index: StringIndex, lang_id: u16) -> Option<&'_ str>;
}

impl<'a, B> InterfaceHList<'a, B> for HNil {
    fn get_id_mut(&mut self, _: u8) -> Option<&mut dyn InterfaceClass<'a, B>> {
        None
    }

    fn get_id(&self, _: u8) -> Option<&dyn InterfaceClass<'a, B>> {
        None
    }

    fn reset(&mut self) {}

    fn write_descriptors(&self, _: &mut DescriptorWriter) -> usb_device::Result<()> {
        Ok(())
    }

    fn get_string(&self, _: StringIndex, _: u16) -> Option<&'static str> {
        None
    }
}

impl<'a, B: UsbBus, Head: InterfaceClass<'a, B> + 'a, Tail: InterfaceHList<'a, B>>
    InterfaceHList<'a, B> for HCons<Head, Tail>
{
    fn get_id_mut(&mut self, id: u8) -> Option<&mut dyn InterfaceClass<'a, B>> {
        if id == u8::from(self.head.id()) {
            Some(&mut self.head)
        } else {
            self.tail.get_id_mut(id)
        }
    }

    fn get_id(&self, id: u8) -> Option<&dyn InterfaceClass<'a, B>> {
        if id == u8::from(self.head.id()) {
            Some(&self.head)
        } else {
            self.tail.get_id(id)
        }
    }

    fn reset(&mut self) {
        self.head.reset();
        self.tail.reset();
    }

    fn write_descriptors(&self, writer: &mut DescriptorWriter) -> usb_device::Result<()> {
        self.head.write_descriptors(writer)?;
        self.tail.write_descriptors(writer)
    }

    fn get_string(&self, index: StringIndex, lang_id: u16) -> Option<&'_ str> {
        let s = self.head.get_string(index, lang_id);
        if s.is_some() {
            s
        } else {
            self.tail.get_string(index, lang_id)
        }
    }
}

pub trait WrappedInterface<'a, B, I, Config = ()>: Sized + InterfaceClass<'a, B>
where
    B: UsbBus,
    I: InterfaceClass<'a, B>,
{
    fn new(interface: I, config: Config) -> Self;
}

pub struct WrappedInterfaceConfig<I, InnerConfig, Config = ()> {
    pub inner_config: InnerConfig,
    pub config: Config,
    interface: PhantomData<I>,
}

impl<I, InnerConfig, Config> WrappedInterfaceConfig<I, InnerConfig, Config> {
    pub fn new(inner_config: InnerConfig, config: Config) -> Self {
        Self {
            inner_config,
            config,
            interface: PhantomData::default(),
        }
    }
}

impl<'a, B, I, InnerConfig, Config> UsbAllocatable<'a, B>
    for WrappedInterfaceConfig<I, InnerConfig, Config>
where
    B: UsbBus + 'a,
    InnerConfig: UsbAllocatable<'a, B>,
    I: WrappedInterface<'a, B, InnerConfig::Allocated, Config>,
    InnerConfig::Allocated: InterfaceClass<'a, B>,
{
    type Allocated = I;

    fn allocate(self, usb_alloc: &'a UsbBusAllocator<B>) -> Self::Allocated {
        I::new(self.inner_config.allocate(usb_alloc), self.config)
    }
}
