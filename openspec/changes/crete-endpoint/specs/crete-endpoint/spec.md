## ADDED Requirements

### Requirement: Endpoint generates UUID with timestamp
The system SHALL generate a unique identifier (UUID) with a timestamp when the `/crete` endpoint is called.

#### Scenario: Successful UUID generation
- **WHEN** a POST request is made to `/crete`
- **THEN** the system returns a 200 OK status with JSON containing `uuid` and `timestamp` fields

#### Scenario: UUID format validation
- **WHEN** the response is received
- **THEN** the `uuid` field contains a valid UUID v4 format (8-4-4-4-12 hexadecimal characters with hyphens)

#### Scenario: Timestamp format validation
- **WHEN** the response is received
- **THEN** the `timestamp` field contains a valid ISO8601 formatted timestamp in UTC

#### Scenario: UUID uniqueness
- **WHEN** multiple requests are made to `/crete`
- **THEN** each response contains a unique UUID
