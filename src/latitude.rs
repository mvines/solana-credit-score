use {
    chrono::{
        prelude::{DateTime, Utc},
        Datelike, Months, NaiveDate, NaiveDateTime, NaiveTime,
    },
    curl::easy::{Easy, List},
    serde_json::Value,
    std::{
        env,
        fmt::{Debug, Formatter},
        time::SystemTime,
    },
    url::form_urlencoded,
};

pub struct Latitude {
    latitude_api_key: Option<String>,
}

#[derive(Default)]
pub struct BandwidthUsage {
    pub inbound: u64,
    pub outbound: u64,
    pub quota: u64,
    pub inbound_usage: u64,
    pub outbound_usage: u64,
    pub days_elapsed: i64,
    pub days_in_month: i64,
}

impl Debug for BandwidthUsage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "\tquota: {} GB\n\
            \tusage: {}% in | {} out%\n\
            \ttime: {} days | {}% month\n",
            self.quota,
            self.inbound_usage,
            self.outbound_usage,
            self.days_elapsed,
            self.days_elapsed * 100 / self.days_in_month,
        )
    }
}

impl Latitude {
    pub fn default() -> Self {
        let latitude_api_key = env::var("LATITUDE_API_KEY").ok();
        Latitude { latitude_api_key }
    }

    pub fn get_traffic_quota(&self) -> Option<(u64, String)> {
        let latitude_api_key = self.latitude_api_key.as_ref()?;

        let mut easy = Easy::new();
        easy.url("https://api.latitude.sh/traffic/quota").unwrap();

        let mut list = List::new();
        list.append(&format!("Authorization: {}", latitude_api_key))
            .unwrap();
        list.append("accept: application/json").unwrap();
        easy.http_headers(list).unwrap();

        let mut json_data: String = String::new();
        {
            let mut transfer = easy.transfer();
            transfer
                .write_function(|data| {
                    json_data.push_str(&String::from_utf8(Vec::from(data)).unwrap());
                    Ok(data.len())
                })
                .unwrap();
            transfer.perform().unwrap();
        }
        let response: Value =
            serde_json::from_str(&json_data).expect("Failed to parse the response as JSON");
        let project_id = &response["data"]["attributes"]["quota_per_project"][0]["project_id"];
        let total_quota = &response["data"]["attributes"]["quota_per_project"][0]
            ["quota_per_region"][0]["quota_in_tb"]["total"];

        project_id.as_str().and_then(|project_id| {
            total_quota
                .as_u64()
                .map(|v| (v * 1024, project_id.to_string()))
        })
    }

    pub fn get_bandwidth_usage(&self) -> Option<BandwidthUsage> {
        let (quota, project_id) = self.get_traffic_quota()?;
        let (start, end, days_elapsed, days_in_month) =
            Latitude::get_date_range(5, &Latitude::get_current_dt_utc())
                .expect("Failed to get start/end dates");
        let start_date: String = form_urlencoded::byte_serialize(start.as_bytes()).collect();
        let end_date: String = form_urlencoded::byte_serialize(end.as_bytes()).collect();

        let latitude_api_key = self.latitude_api_key.as_ref()?;
        let url = format!("https://api.latitude.sh/traffic?filter[project]={}&filter[date][gte]={}Z&filter[date][lte]={}Z", project_id, start_date, end_date);

        let mut easy = Easy::new();
        easy.url(&url).unwrap();

        let mut list = List::new();
        list.append(&format!("Authorization: {}", latitude_api_key))
            .unwrap();
        list.append("accept: application/json").unwrap();
        easy.http_headers(list).unwrap();

        let mut json_data: String = String::new();
        {
            let mut transfer = easy.transfer();
            transfer
                .write_function(|data| {
                    json_data.push_str(&String::from_utf8(Vec::from(data)).unwrap());
                    Ok(data.len())
                })
                .unwrap();
            transfer.perform().unwrap();
        }
        let response: Value =
            serde_json::from_str(&json_data).expect("Failed to parse the response as JSON");
        let inbound_value = &response["data"]["attributes"]["total_inbound_gb"];
        let outbound_value = &response["data"]["attributes"]["total_outbound_gb"];

        inbound_value
            .as_u64()
            .and_then(|v| outbound_value.as_u64().map(|o| (v, o)))
            .map(|(inbound, outbound)| BandwidthUsage {
                inbound,
                outbound,
                quota,
                inbound_usage: inbound * 100 / quota,
                outbound_usage: outbound * 100 / quota,
                days_elapsed,
                days_in_month,
            })
    }

    pub fn get_date_range(
        start_day: u32,
        reference_date: &DateTime<Utc>,
    ) -> Option<(String, String, i64, i64)> {
        let month = reference_date.month();
        let year = reference_date.year();
        let day = reference_date.day();
        let native_date_time = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(year, month, start_day)?,
            NaiveTime::default(),
        );
        let utc_date = DateTime::<Utc>::from_naive_utc_and_offset(
            native_date_time,
            reference_date.offset().clone(),
        );

        let (start, end) = if day < start_day {
            (utc_date.checked_sub_months(Months::new(1))?, utc_date)
        } else {
            (utc_date, utc_date.checked_add_months(Months::new(1))?)
        };

        let days_elapsed = (*reference_date - start).num_days();
        let days_in_month = (end - start).num_days();

        Some((
            format!("{}", start.format("%Y-%m-%dT00:00:00")),
            format!("{}", end.format("%Y-%m-%dT00:00:00")),
            days_elapsed,
            days_in_month,
        ))
    }

    pub fn get_current_dt_utc() -> DateTime<Utc> {
        SystemTime::now().into()
    }
}

#[cfg(test)]
mod test {
    use {
        crate::latitude::Latitude,
        chrono::{DateTime, Utc},
        std::str::FromStr,
    };

    #[test]
    fn test_date_range() {
        let reference = DateTime::<Utc>::from_str("2024-08-12T00:00:00+00:00")
            .expect("Failed to create reference date");
        let (start, end, days_elapsed, days_in_the_month) =
            Latitude::get_date_range(5, &reference).expect("Failed to compute dates");
        assert_eq!(start, "2024-08-05T00:00:00");
        assert_eq!(end, "2024-09-05T00:00:00");
        assert_eq!(days_elapsed, 7);
        assert_eq!(days_in_the_month, 31);

        let reference = DateTime::<Utc>::from_str("2024-08-12T00:00:00+00:00")
            .expect("Failed to create reference date");
        let (start, end, days_elapsed, days_in_the_month) =
            Latitude::get_date_range(25, &reference).expect("Failed to compute dates");
        assert_eq!(start, "2024-07-25T00:00:00");
        assert_eq!(end, "2024-08-25T00:00:00");
        assert_eq!(days_elapsed, 18);
        assert_eq!(days_in_the_month, 31);

        let reference = DateTime::<Utc>::from_str("2024-12-12T00:00:00+00:00")
            .expect("Failed to create reference date");
        let (start, end, days_elapsed, days_in_the_month) =
            Latitude::get_date_range(5, &reference).expect("Failed to compute dates");
        assert_eq!(start, "2024-12-05T00:00:00");
        assert_eq!(end, "2025-01-05T00:00:00");
        assert_eq!(days_elapsed, 7);
        assert_eq!(days_in_the_month, 31);

        let reference = DateTime::<Utc>::from_str("2024-01-12T00:00:00+00:00")
            .expect("Failed to create reference date");
        let (start, end, days_elapsed, days_in_the_month) =
            Latitude::get_date_range(25, &reference).expect("Failed to compute dates");
        assert_eq!(start, "2023-12-25T00:00:00");
        assert_eq!(end, "2024-01-25T00:00:00");
        assert_eq!(days_elapsed, 18);
        assert_eq!(days_in_the_month, 31);

        let (start, end, _days_elapsed, _days_in_the_month) =
            Latitude::get_date_range(5, &Latitude::get_current_dt_utc())
                .expect("Failed to compute dates");
        println!("For current time, start {}, end {}", start, end);
    }

    #[test]
    fn test_bandwidth_usage() {
        let mut latitude = Latitude::default();
        latitude.latitude_api_key = Some("".to_string());

        let usage = latitude.get_bandwidth_usage();
        println!("Usage is {:?}", usage);
    }

    #[test]
    fn test_get_traffic_quota() {
        let mut latitude = Latitude::default();
        latitude.latitude_api_key = Some("".to_string());

        let quota = latitude.get_traffic_quota();
        println!("Quota is {:?}", quota);
    }
}
