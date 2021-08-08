use crate::error::Error;
use std::collections::VecDeque;
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::ptr::null;
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::Duration;
use std::{mem, slice, thread};

use core_foundation_sys::string::{CFStringGetCString, CFStringGetCStringPtr, CFStringRef};
use sys;
use sys::{
    kAudioDevicePropertyAvailableNominalSampleRates, kAudioDevicePropertyDeviceNameCFString,
    kAudioDevicePropertyNominalSampleRate, kAudioDevicePropertyScopeOutput, kAudioHardwareNoError,
    kAudioHardwarePropertyDefaultInputDevice, kAudioHardwarePropertyDefaultOutputDevice,
    kAudioHardwarePropertyDevices, kAudioObjectPropertyElementMaster,
    kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeInput,
    kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
    kAudioOutputUnitProperty_CurrentDevice, kAudioOutputUnitProperty_EnableIO,
    kAudioStreamPropertyAvailablePhysicalFormats, kAudioStreamPropertyPhysicalFormat,
    kCFStringEncodingUTF8, AudioDeviceID, AudioObjectAddPropertyListener,
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectRemovePropertyListener, AudioObjectSetPropertyData,
    AudioStreamBasicDescription, AudioValueRange, OSStatus,
};

use crate::audio_unit::{AudioUnit, Element, IOType, Scope};

/// Helper function to get the device id of the default input or output device
#[cfg(target_os = "macos")]
pub fn get_default_device_id(input: bool) -> Option<AudioDeviceID> {
    let selector = if input {
        kAudioHardwarePropertyDefaultInputDevice
    } else {
        kAudioHardwarePropertyDefaultOutputDevice
    };
    let property_address = AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMaster,
    };

    let audio_device_id: AudioDeviceID = 0;
    let data_size = mem::size_of::<AudioDeviceID>();
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &property_address as *const _,
            0,
            null(),
            &data_size as *const _ as *mut _,
            &audio_device_id as *const _ as *mut _,
        )
    };
    if status != kAudioHardwareNoError as i32 {
        return None;
    }

    Some(audio_device_id)
}

/// Create an AudioUnit instance from a device id.
#[cfg(target_os = "macos")]
pub fn audio_unit_from_device_id(
    device_id: AudioDeviceID,
    input: bool,
) -> Result<AudioUnit, Error> {
    let mut audio_unit = AudioUnit::new(IOType::HalOutput)?;

    if input {
        // Enable input processing.
        let enable_input = 1u32;
        audio_unit.set_property(
            kAudioOutputUnitProperty_EnableIO,
            Scope::Input,
            Element::Input,
            Some(&enable_input),
        )?;

        // Disable output processing.
        let disable_output = 0u32;
        audio_unit.set_property(
            kAudioOutputUnitProperty_EnableIO,
            Scope::Output,
            Element::Output,
            Some(&disable_output),
        )?;
    }

    audio_unit.set_property(
        kAudioOutputUnitProperty_CurrentDevice,
        Scope::Global,
        Element::Output,
        Some(&device_id),
    )?;

    Ok(audio_unit)
}

/// Helper to list all audio device ids on the system.
#[cfg(target_os = "macos")]
pub fn get_audio_device_ids() -> Result<Vec<AudioDeviceID>, Error> {
    let property_address = AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyDevices,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMaster,
    };

    macro_rules! try_status_or_return {
        ($status:expr) => {
            if $status != kAudioHardwareNoError as i32 {
                return Err(Error::Unknown($status));
            }
        };
    }

    let data_size = 0u32;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            kAudioObjectSystemObject,
            &property_address as *const _,
            0,
            null(),
            &data_size as *const _ as *mut _,
        )
    };
    try_status_or_return!(status);

    let device_count = data_size / mem::size_of::<AudioDeviceID>() as u32;
    let mut audio_devices = vec![];
    audio_devices.reserve_exact(device_count as usize);

    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject,
            &property_address as *const _,
            0,
            null(),
            &data_size as *const _ as *mut _,
            audio_devices.as_mut_ptr() as *mut _,
        )
    };
    try_status_or_return!(status);

    unsafe { audio_devices.set_len(device_count as usize) };

    Ok(audio_devices)
}

/// Get the device name for the device id.
#[cfg(target_os = "macos")]
pub fn get_device_name(device_id: AudioDeviceID) -> Result<String, Error> {
    let property_address = AudioObjectPropertyAddress {
        mSelector: kAudioDevicePropertyDeviceNameCFString,
        mScope: kAudioDevicePropertyScopeOutput,
        mElement: kAudioObjectPropertyElementMaster,
    };

    macro_rules! try_status_or_return {
        ($status:expr) => {
            if $status != kAudioHardwareNoError as i32 {
                return Err(Error::Unknown($status));
            }
        };
    }

    let device_name: CFStringRef = null();
    let data_size = mem::size_of::<CFStringRef>();
    let c_str = unsafe {
        let status = AudioObjectGetPropertyData(
            device_id,
            &property_address as *const _,
            0,
            null(),
            &data_size as *const _ as *mut _,
            &device_name as *const _ as *mut _,
        );
        try_status_or_return!(status);

        let c_string: *const c_char = CFStringGetCStringPtr(device_name, kCFStringEncodingUTF8);
        if c_string.is_null() {
            let status = AudioObjectGetPropertyData(
                device_id,
                &property_address as *const _,
                0,
                null(),
                &data_size as *const _ as *mut _,
                &device_name as *const _ as *mut _,
            );
            try_status_or_return!(status);
            let mut buf: [i8; 255] = [0; 255];
            let result = CFStringGetCString(
                device_name,
                buf.as_mut_ptr(),
                buf.len() as _,
                kCFStringEncodingUTF8,
            );
            if result == 0 {
                return Err(Error::Unknown(result as i32));
            }
            let name: &CStr = CStr::from_ptr(buf.as_ptr());
            return Ok(name.to_str().unwrap().to_owned());
        }
        CStr::from_ptr(c_string as *mut _)
    };
    Ok(c_str.to_string_lossy().into_owned())
}

/// Change the sample rate of a device.
/// Adapted from CPAL.
#[cfg(target_os = "macos")]
pub fn set_device_sample_rate(device_id: AudioDeviceID, new_rate: f64) -> Result<(), Error> {
    // Check whether or not we need to change the device sample rate to suit the one specified for the stream.
    unsafe {
        // Get the current sample rate.
        let mut property_address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyNominalSampleRate,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMaster,
        };
        let sample_rate: f64 = 0.0;
        let data_size = mem::size_of::<f64>() as u32;
        let status = AudioObjectGetPropertyData(
            device_id,
            &property_address as *const _,
            0,
            null(),
            &data_size as *const _ as *mut _,
            &sample_rate as *const _ as *mut _,
        );
        Error::from_os_status(status)?;

        // If the requested sample rate is different to the device sample rate, update the device.
        if sample_rate as u32 != new_rate as u32 {
            // Get available sample rate ranges.
            property_address.mSelector = kAudioDevicePropertyAvailableNominalSampleRates;
            let data_size = 0u32;
            let status = AudioObjectGetPropertyDataSize(
                device_id,
                &property_address as *const _,
                0,
                null(),
                &data_size as *const _ as *mut _,
            );
            Error::from_os_status(status)?;
            let n_ranges = data_size as usize / mem::size_of::<AudioValueRange>();
            let mut ranges: Vec<u8> = vec![];
            ranges.reserve_exact(data_size as usize);
            let status = AudioObjectGetPropertyData(
                device_id,
                &property_address as *const _,
                0,
                null(),
                &data_size as *const _ as *mut _,
                ranges.as_mut_ptr() as *mut _,
            );
            Error::from_os_status(status)?;
            let ranges: *mut AudioValueRange = ranges.as_mut_ptr() as *mut _;
            let ranges: &'static [AudioValueRange] = slice::from_raw_parts(ranges, n_ranges);

            // Now that we have the available ranges, pick the one matching the desired rate.
            let new_rate_integer = new_rate as u32;
            let maybe_index = ranges.iter().position(|r| {
                r.mMinimum as u32 == new_rate_integer && r.mMaximum as u32 == new_rate_integer
            });
            let range_index = match maybe_index {
                None => return Err(Error::UnsupportedSampleRate),
                Some(i) => i,
            };

            // Update the property selector to specify the nominal sample rate.
            property_address.mSelector = kAudioDevicePropertyNominalSampleRate;

            // Add a listener to know when the sample rate changes.
            // Since the listener implements Drop, we don't need to manually unregister this later.
            let (sender, receiver) = channel();
            let mut listener = RateListener::new(device_id, Some(sender))?;
            listener.register()?;

            // Finally, set the sample rate.
            let status = AudioObjectSetPropertyData(
                device_id,
                &property_address as *const _,
                0,
                null(),
                data_size,
                &ranges[range_index] as *const _ as *const _,
            );
            Error::from_os_status(status)?;

            // Wait for the reported_rate to change.
            //
            // This should not take longer than a few ms, but we timeout after 1 sec just in case.
            let timer = ::std::time::Instant::now();
            loop {
                println!("waiting for rate change");
                if let Ok(reported_rate) = receiver.recv_timeout(Duration::from_millis(100)) {
                    println!("got rate change event");
                    if new_rate as usize == reported_rate as usize {
                        println!("rate was updated!");
                        break;
                    }
                }
                /*
                if listener.get_nbr_values() > 0 {
                    if let Some(reported_rate) = listener.copy_values().last() {
                        if new_rate as usize == *reported_rate as usize {
                            println!("rate was updated!");
                            break;
                        }
                    }
                }
                */
                if timer.elapsed() > Duration::from_secs(1) {
                    return Err(Error::UnsupportedSampleRate);
                }
                //thread::sleep(Duration::from_millis(5));
            }
        };
        Ok(())
    }
}

/// Change the sample rate and format of a device.
#[cfg(target_os = "macos")]
pub fn set_device_sample_format(
    device_id: AudioDeviceID,
    new_asbd: AudioStreamBasicDescription,
) -> Result<(), Error> {
    // Check whether or not we need to change the device sample format and rate.
    unsafe {
        // Get the current sample rate.
        let property_address = AudioObjectPropertyAddress {
            mSelector: kAudioStreamPropertyPhysicalFormat,
            mScope: kAudioObjectPropertyScopeGlobal,
            //mScope: kAudioDevicePropertyScopeInput,
            //mScope: kAudioObjectPropertyScopeOutput,
            //mScope: kAudioDevicePropertyScopeOutput,
            mElement: kAudioObjectPropertyElementMaster,
        };
        let maybe_asbd: mem::MaybeUninit<AudioStreamBasicDescription> = mem::MaybeUninit::zeroed();
        let data_size = mem::size_of::<AudioStreamBasicDescription>() as u32;
        let status = AudioObjectGetPropertyData(
            device_id,
            &property_address as *const _,
            0,
            null(),
            &data_size as *const _ as *mut _,
            &maybe_asbd as *const _ as *mut _,
        );
        Error::from_os_status(status)?;
        let asbd = maybe_asbd.assume_init();
        println!("---- Current format ----");
        println!("{:#?}", asbd);
        //println!("{:#?}", StreamFormat::from_asbd(asbd).unwrap());

        // If the requested sample rate and/or format is different to the device sample rate, update the device.
        if !asbds_are_equal(&asbd, &new_asbd) {
            let property_address = AudioObjectPropertyAddress {
                mSelector: kAudioStreamPropertyPhysicalFormat,
                mScope: kAudioObjectPropertyScopeGlobal,
                mElement: kAudioObjectPropertyElementMaster,
            };

            let reported_asbd: mem::MaybeUninit<AudioStreamBasicDescription> =
                mem::MaybeUninit::zeroed();
            let reported_asbd = reported_asbd.assume_init();

            let status = AudioObjectSetPropertyData(
                device_id,
                &property_address as *const _,
                0,
                null(),
                data_size,
                &new_asbd as *const _ as *const _,
            );
            Error::from_os_status(status)?;

            // Wait for the reported format to change.
            //
            // This should not take longer than a few ms, but we timeout after 1 sec just in case.
            println!("{:#?}", reported_asbd);
            let timer = ::std::time::Instant::now();
            loop {
                let status = AudioObjectGetPropertyData(
                    device_id,
                    &property_address as *const _,
                    0,
                    null(),
                    &data_size as *const _ as *mut _,
                    &reported_asbd as *const _ as *mut _,
                );
                Error::from_os_status(status)?;
                if asbds_are_equal(&reported_asbd, &new_asbd) {
                    break;
                }
                println!("spinning");
                thread::sleep(Duration::from_millis(5));
                if timer.elapsed() > Duration::from_secs(1) {
                    return Err(Error::UnsupportedSampleRate);
                }
            }
            println!("{:#?}", reported_asbd);
        }
        Ok(())
    }
}

/// Helper to check if two ASBDs are equal.
fn asbds_are_equal(
    left: &AudioStreamBasicDescription,
    right: &AudioStreamBasicDescription,
) -> bool {
    left.mSampleRate as u32 == right.mSampleRate as u32
        && left.mFormatID == right.mFormatID
        && left.mFormatFlags == right.mFormatFlags
        && left.mBytesPerPacket == right.mBytesPerPacket
        && left.mFramesPerPacket == right.mFramesPerPacket
        && left.mBytesPerFrame == right.mBytesPerFrame
        && left.mChannelsPerFrame == right.mChannelsPerFrame
        && left.mBitsPerChannel == right.mBitsPerChannel
}

/// Get a vector with all supported formats as AudioBasicStreamDescriptions.
#[cfg(target_os = "macos")]
pub fn get_supported_stream_formats(
    device_id: AudioDeviceID,
) -> Result<Vec<AudioStreamBasicDescription>, Error> {
    // Get available formats.
    let mut property_address = AudioObjectPropertyAddress {
        mSelector: kAudioStreamPropertyPhysicalFormat,
        mScope: kAudioObjectPropertyScopeGlobal,
        //mScope: kAudioDevicePropertyScopeInput,
        //mScope: kAudioObjectPropertyScopeOutput,
        //mScope: kAudioDevicePropertyScopeOutput,
        mElement: kAudioObjectPropertyElementMaster,
    };
    let formats = unsafe {
        property_address.mSelector = kAudioStreamPropertyAvailablePhysicalFormats;
        let data_size = 0u32;
        let status = AudioObjectGetPropertyDataSize(
            device_id,
            &property_address as *const _,
            0,
            null(),
            &data_size as *const _ as *mut _,
        );
        Error::from_os_status(status)?;
        let n_formats = data_size as usize / mem::size_of::<AudioStreamBasicDescription>();
        let mut formats: Vec<u8> = vec![];
        formats.reserve_exact(data_size as usize);
        let status = AudioObjectGetPropertyData(
            device_id,
            &property_address as *const _,
            0,
            null(),
            &data_size as *const _ as *mut _,
            formats.as_mut_ptr() as *mut _,
        );
        Error::from_os_status(status)?;
        let formats: *mut AudioStreamBasicDescription = formats.as_mut_ptr() as *mut _;
        Vec::from_raw_parts(formats, n_formats, n_formats)
    };
    /*
    println!("---- All supported formats ----");
    for asbd in formats.iter() {
        if let Ok(sf) = StreamFormat::from_asbd(*asbd) {
                println!("{:#?}", asbd);
                println!("{:#?}", sf);
        }
    }
    */
    Ok(formats)
}

/// Changing the sample rate is an asynchonous process.
/// Use a RateListener to get notified when the rate is changed.
#[cfg(target_os = "macos")]
pub struct RateListener {
    pub queue: Mutex<VecDeque<f64>>,
    sync_channel: Option<Sender<f64>>,
    device_id: AudioDeviceID,
    property_address: AudioObjectPropertyAddress,
    rate_listener: Option<
        unsafe extern "C" fn(u32, u32, *const AudioObjectPropertyAddress, *mut c_void) -> i32,
    >,
}

#[cfg(target_os = "macos")]
impl Drop for RateListener {
    fn drop(&mut self) {
        println!("Dropping RateListener!");
        let _ = self.unregister();
    }
}

#[cfg(target_os = "macos")]
impl RateListener {
    /// Create a new RateListener for the given AudioDeviceID.
    /// If a sync Sender is provided, then events will be pushed to that channel.
    /// If not, they will be stored in an internal queue that will need to be polled.
    pub fn new(
        device_id: AudioDeviceID,
        sync_channel: Option<Sender<f64>>,
    ) -> Result<RateListener, Error> {
        // Add our sample rate change listener callback.
        let property_address = AudioObjectPropertyAddress {
            mSelector: kAudioDevicePropertyNominalSampleRate,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMaster,
        };
        let queue = Mutex::new(VecDeque::new());
        Ok(RateListener {
            queue,
            sync_channel,
            device_id,
            property_address,
            rate_listener: None,
        })
    }

    /// Register this listener to receive notifications.
    pub fn register(&mut self) -> Result<(), Error> {
        unsafe extern "C" fn rate_listener(
            device_id: AudioObjectID,
            _n_addresses: u32,
            _properties: *const AudioObjectPropertyAddress,
            self_ptr: *mut ::std::os::raw::c_void,
        ) -> OSStatus {
            let self_ptr: &mut RateListener = &mut *(self_ptr as *mut RateListener);
            let rate: f64 = 0.0;
            let data_size = mem::size_of::<f64>();
            let property_address = AudioObjectPropertyAddress {
                mSelector: kAudioDevicePropertyNominalSampleRate,
                mScope: kAudioObjectPropertyScopeGlobal,
                mElement: kAudioObjectPropertyElementMaster,
            };
            let result = AudioObjectGetPropertyData(
                device_id,
                &property_address as *const _,
                0,
                null(),
                &data_size as *const _ as *mut _,
                &rate as *const _ as *mut _,
            );
            if let Some(sender) = &self_ptr.sync_channel {
                sender.send(rate).unwrap();
            } else {
                let mut queue = self_ptr.queue.lock().unwrap();
                queue.push_back(rate);
            }
            result
        }

        // Add our sample rate change listener callback.
        let status = unsafe {
            AudioObjectAddPropertyListener(
                self.device_id,
                &self.property_address as *const _,
                Some(rate_listener),
                self as *const _ as *mut _,
            )
        };
        Error::from_os_status(status)?;
        self.rate_listener = Some(rate_listener);
        Ok(())
    }

    /// Unregister this listener to stop receiving notifications
    pub fn unregister(&mut self) -> Result<(), Error> {
        // Add our sample rate change listener callback.
        if self.rate_listener.is_some() {
            let status = unsafe {
                AudioObjectRemovePropertyListener(
                    self.device_id,
                    &self.property_address as *const _,
                    self.rate_listener,
                    self as *const _ as *mut _,
                )
            };
            Error::from_os_status(status)?;
            self.rate_listener = None;
        }
        Ok(())
    }

    /// Get the number of sample rate values received (equals the number of change events).
    pub fn get_nbr_values(&self) -> usize {
        self.queue.lock().unwrap().len()
    }

    /// Copy all received values to a Vec. The latest value is the last element.
    /// The internal buffer is preserved.
    pub fn copy_values(&self) -> Vec<f64> {
        self.queue
            .lock()
            .unwrap()
            .iter()
            .copied()
            .collect::<Vec<f64>>()
    }

    /// Get all received values as a Vec. The latest value is the last element.
    /// This clears the internal buffer.
    pub fn drain_values(&mut self) -> Vec<f64> {
        self.queue.lock().unwrap().drain(..).collect::<Vec<f64>>()
    }
}